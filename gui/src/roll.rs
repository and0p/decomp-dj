//! The piano roll: time runs left to right, one row per button, rows grouped by
//! controller. Groups that the replay uses are highlighted and open; unused
//! groups are dimmed and collapsed. Only binary inputs (buttons and keys) get
//! rows for now; other input lines show as ticks on their group's header.
//!
//! Spans are worked out once per replay edit and drawn only where visible, so
//! long replays with many presses stay cheap to draw.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind};
use decomp_dj::collection::Collection;
use decomp_dj::format::{self, Command, InputKey, DEVICE_JOYPAD, DEVICE_KEYBOARD, DEVICE_MOUSE};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const LABEL_W: f32 = 132.0;
const RULER_H: f32 = 20.0;
const MARK_H: f32 = 18.0;
/// Longest marker label drawn in the lane, in characters.
const LABEL_CHARS: usize = 40;
const GROUP_H: f32 = 20.0;
const ROW_H: f32 = 15.0;
/// Joypad rows in display order: d-pad, SELECT, START, B, A, then the rest.
const JOYPAD_ORDER: [u32; 16] = [4, 5, 6, 7, 2, 3, 0, 8, 1, 9, 10, 11, 12, 13, 14, 15];
const JOYPAD_BASIC: usize = 8;

/// What the roll shows around the segment, read-only: the end of the parent to the left of
/// frame 0 and the start of the next segment past the last frame; and from which frame
/// on the segment's hashes aren't known, as a band on the ruler in its status colour.
pub struct Neighbours {
    pub prev: Option<String>,
    pub next: Option<String>,
    pub dirty: Option<DirtyBand>,
}

/// The ruler band: frames from `from` on aren't verified. `hint` is the hover text.
#[derive(Clone)]
pub struct DirtyBand {
    pub from: u64,
    pub color: Color32,
    pub hint: String,
}

pub enum Action {
    Seek(u64),
    /// Open the editor for the non-input lines on this frame.
    OpenFrame(u64),
    /// New spans for one or more buttons; `from` is the first frame whose input changed.
    Edit { edits: Vec<(InputKey, Vec<(u64, u64)>)>, from: u64 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelOp {
    Replace,
    Add,
    Remove,
}

/// A press is named by its button and first frame.
type PressId = (InputKey, u64);

/// A drag in select mode. Frame positions are fractional frames, so the
/// view can scroll under a drag without losing its anchor.
enum Gesture {
    /// Moving the selected presses in time, and with Alt held, to other rows of the same
    /// controller (`dr` rows down). `click` is the press under the pointer, which becomes
    /// the only selection if the pointer never moves. `from` is that press's row.
    Move { anchor: f32, delta: i64, click: Option<PressId>, from: (usize, usize), dr: i64 },
    /// Dragging one edge of one press.
    Resize { key: InputKey, span: (u64, u64), left: bool, to: u64 },
    /// Selecting by area: corners in (frame, content y).
    /// `cell` is the empty cell the drag began on (group, row, frame): a click that
    /// doesn't move adds a press there.
    Marquee { a: (f32, f32), b: (f32, f32), op: SelOp, cell: Option<(usize, usize, u64)> },
}

struct Row {
    key: InputKey,
    label: String,
    spans: Vec<(u64, u64)>,
}

struct Group {
    port: u32,
    device: u32,
    name: String,
    rows: Vec<Row>,
    /// All rows' spans merged, drawn on the header.
    union: Vec<(u64, u64)>,
    /// Frames with non-binary input lines (axes, pointer positions).
    other: Vec<u64>,
    presses: usize,
}

impl Group {
    fn used(&self) -> bool {
        self.presses > 0 || !self.other.is_empty()
    }
}

struct Marker {
    frame: u64,
    glyph: &'static str,
    color: Color32,
    text: String,
    /// Short text shown in the lane.
    label: String,
}

struct Model {
    id: String,
    gen: u64,
    all_buttons: bool,
    frames: u64,
    groups: Vec<Group>,
    markers: Vec<Marker>,
    prev: Option<Side>,
    next: Option<Side>,
}

/// A neighbouring segment, laid out on this segment's rows: `rows[g][r]` are the spans of
/// row `r` of group `g`, and `union[g]` everything the neighbour presses on that controller.
struct Side {
    id: String,
    title: String,
    frames: u64,
    rows: Vec<Vec<Vec<(u64, u64)>>>,
    union: Vec<Vec<(u64, u64)>>,
}

fn build_side(col: &Collection, id: &str, groups: &[Group]) -> Option<Side> {
    let r = col.replays.get(id)?;
    let mut keys: BTreeSet<InputKey> = BTreeSet::new();
    for l in &r.lines {
        if let Command::Input { key, .. } = &l.command {
            if format::is_binary(*key) {
                keys.insert(*key);
            }
        }
    }
    let rows = groups.iter().map(|g| g.rows.iter().map(|row| if keys.contains(&row.key) { r.binary_spans(row.key) } else { vec![] }).collect()).collect();
    let union = groups
        .iter()
        .map(|g| merge(keys.iter().filter(|k| k.port == g.port && k.device == g.device).flat_map(|k| r.binary_spans(*k)).collect()))
        .collect();
    Some(Side { id: id.to_string(), title: r.title().to_string(), frames: r.frames().unwrap_or(0), rows, union })
}

/// Making a new press: from a click on an empty cell. A plain click makes it as long as
/// the last press made or resized; dragging sets its length.
struct Drag {
    group: usize,
    row: usize,
    anchor: u64,
    current: u64,
    moved: bool,
}

impl Drag {
    fn span(&self, last_len: u64, frames: u64) -> (u64, u64) {
        let (a, b) = if self.moved { (self.anchor.min(self.current), self.anchor.max(self.current) + 1) } else { (self.anchor, self.anchor + last_len.max(1)) };
        (a, b.min(frames.max(a + 1)))
    }
}

pub struct RollState {
    pub px_per_frame: f32,
    /// Frame at the left edge of the timeline.
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub follow: bool,
    pub all_buttons: bool,
    /// Show marker text in the lane (otherwise only symbols).
    pub labels: bool,
    /// Marker lane expanded: every marker gets room for its whole label,
    /// events in the top rows and everything else (RAM writes, resets, ...) below.
    pub markers_open: bool,
    /// Edit > Drag to draw inputs: a drag on an empty cell draws a press (otherwise it
    /// selects an area). Ctrl/Cmd does the other one.
    pub draw: bool,
    open: HashMap<(u32, u32), bool>,
    gen: u64,
    model: Option<Model>,
    drag: Option<Drag>,
    /// Length of the last press made or resized, used for a new press made by a click.
    last_len: u64,
    /// Selected presses.
    sel: BTreeSet<PressId>,
    gesture: Option<Gesture>,
    scrubbing: bool,
    jump_to: Option<u64>,
    /// Record mode is on: the ruler is tinted red and the roll's own keys (Delete,
    /// Backspace, Escape, Alt+Up/Down) are left to recording.
    pub recording: bool,
}

impl Default for RollState {
    fn default() -> Self {
        RollState { px_per_frame: 4.0, scroll_x: 0.0, scroll_y: 0.0, follow: true, all_buttons: false, labels: true, markers_open: false, draw: false, open: HashMap::new(), gen: 0, model: None, drag: None, last_len: 4, sel: BTreeSet::new(), gesture: None, scrubbing: false, jump_to: None, recording: false }
    }
}

fn port_color(port: u32, device: u32) -> Color32 {
    if device != DEVICE_JOYPAD {
        return Color32::from_rgb(235, 150, 60);
    }
    match port {
        0 => Color32::from_rgb(80, 150, 255),
        1 => Color32::from_rgb(240, 90, 100),
        2 => Color32::from_rgb(90, 200, 120),
        _ => Color32::from_rgb(190, 120, 240),
    }
}

fn device_name(d: u32) -> &'static str {
    match d {
        DEVICE_JOYPAD => "Joypad",
        DEVICE_MOUSE => "Mouse",
        DEVICE_KEYBOARD => "Keyboard",
        format::DEVICE_LIGHTGUN => "Lightgun",
        format::DEVICE_ANALOG => "Analog",
        format::DEVICE_POINTER => "Pointer",
        _ => "Device",
    }
}

fn key_label(key: InputKey) -> String {
    match key.device {
        DEVICE_JOYPAD if key.id < 16 => format::JOYPAD[key.id as usize].to_string(),
        DEVICE_MOUSE => format::mouse_name(key.id).unwrap_or("?").to_string(),
        _ => {
            let t = format::input_text(key, 1);
            let t = t.strip_suffix(" 1").unwrap_or(&t);
            t.split_once(' ').map_or(t, |(_, rest)| rest).to_string()
        }
    }
}

pub fn merge(mut v: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    v.sort();
    let mut out: Vec<(u64, u64)> = vec![];
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn build_model(col: &Collection, id: &str, gen: u64, all_buttons: bool) -> Model {
    let r = &col.replays[id];
    let frames = r.frames().unwrap_or(0);
    let mut used: BTreeSet<InputKey> = BTreeSet::new();
    let mut other: BTreeMap<(u32, u32), Vec<u64>> = BTreeMap::new();
    let mut markers = vec![];
    let mut groups_wanted: BTreeSet<(u32, u32)> = BTreeSet::new();
    // Joypads on the collection's ports, and always at least two of them.
    for p in &col.config.ports {
        if let Ok(d) = format::device_from_name(&p.device) {
            if d & 0xff == DEVICE_JOYPAD {
                groups_wanted.insert((p.port, DEVICE_JOYPAD));
            }
        }
    }
    groups_wanted.insert((0, DEVICE_JOYPAD));
    groups_wanted.insert((1, DEVICE_JOYPAD));
    for l in &r.lines {
        match &l.command {
            Command::Input { key, .. } => {
                groups_wanted.insert((key.port, key.device));
                if format::is_binary(*key) {
                    used.insert(*key);
                } else {
                    other.entry((key.port, key.device)).or_default().push(l.frame);
                }
            }
            Command::Device { port, device } => markers.push(Marker { frame: l.frame, glyph: "◆", color: Color32::from_rgb(120, 200, 230), text: format!("[DEVICE] P{} = {device}", port + 1), label: format!("P{} dev {device}", port + 1) }),
            Command::Write { region, address, bytes } => {
                let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                markers.push(Marker { frame: l.frame, glyph: "✏", color: Color32::from_rgb(200, 120, 220), text: format!("[WRITE] {region} 0x{address:X} {}", hex.join(" ")), label: format!("{region} {address:X}={}", hex.join(" ")) })
            }
            Command::Event { text, .. } => markers.push(Marker { frame: l.frame, glyph: "⚑", color: Color32::from_rgb(240, 210, 90), text: format!("[EVENT] {text}"), label: text.clone() }),
            Command::Reset => markers.push(Marker { frame: l.frame, glyph: "⟲", color: Color32::from_rgb(230, 90, 80), text: "[RESET]".into(), label: "reset".into() }),
            Command::Screenshot => markers.push(Marker { frame: l.frame, glyph: "▣", color: Color32::from_gray(200), text: "[SCREENSHOT]".into(), label: String::new() }),
            Command::Rng { name, count } => markers.push(Marker { frame: l.frame, glyph: "🎲", color: Color32::from_gray(200), text: format!("[RNG] {name} {count}"), label: format!("rng {name}") }),
        }
    }
    for (f, h) in r.checkpoints().unwrap_or_default() {
        markers.push(Marker { frame: f, glyph: "✔", color: Color32::from_rgb(92, 184, 92), text: format!("checkpoint {h}"), label: String::new() });
    }
    let mut groups = vec![];
    for (port, device) in groups_wanted {
        let ids: Vec<u32> = match device {
            DEVICE_JOYPAD => JOYPAD_ORDER.iter().enumerate().filter(|(i, id)| all_buttons || *i < JOYPAD_BASIC || used.contains(&InputKey { port, device, index: 0, id: **id })).map(|(_, id)| *id).collect(),
            DEVICE_MOUSE => {
                let mut v = vec![2, 3, 6];
                v.extend(used.iter().filter(|k| k.port == port && k.device == device && !v.contains(&k.id)).map(|k| k.id).collect::<Vec<_>>());
                v
            }
            _ => used.iter().filter(|k| k.port == port && k.device == device).map(|k| k.id).collect(),
        };
        let mut rows = vec![];
        for id in ids {
            // Keyboard and other devices may use index; take whichever keys appear.
            let keys: Vec<InputKey> = {
                let found: Vec<InputKey> = used.iter().filter(|k| k.port == port && k.device == device && k.id == id).copied().collect();
                if found.is_empty() {
                    vec![InputKey { port, device, index: 0, id }]
                } else {
                    found
                }
            };
            for key in keys {
                let spans = r.binary_spans(key);
                rows.push(Row { key, label: key_label(key), spans });
            }
        }
        let presses = rows.iter().map(|r| r.spans.len()).sum();
        let union = merge(rows.iter().flat_map(|r| r.spans.iter().copied()).collect());
        let mut oth = other.remove(&(port, device)).unwrap_or_default();
        oth.dedup();
        groups.push(Group { port, device, name: format!("P{} {}", port + 1, device_name(device)), rows, union, other: oth, presses });
    }
    markers.sort_by_key(|m| m.frame);
    Model { id: id.to_string(), gen, all_buttons, frames, groups, markers, prev: None, next: None }
}

/// Spans overlapping `[f0, f1)`, found by binary search.
fn visible(spans: &[(u64, u64)], f0: u64, f1: u64) -> &[(u64, u64)] {
    let a = spans.partition_point(|s| s.1 <= f0);
    let b = spans.partition_point(|s| s.0 < f1);
    &spans[a..b.max(a)]
}

fn row_of(m: &Model, key: InputKey) -> Option<(usize, usize)> {
    m.groups.iter().enumerate().find_map(|(gi, g)| g.rows.iter().position(|r| r.key == key).map(|ri| (gi, ri)))
}

/// The span of `spans` that contains frame `f`.
fn span_at(spans: &[(u64, u64)], f: u64) -> Option<(u64, u64)> {
    spans.iter().copied().find(|s| s.0 <= f && f < s.1)
}

/// A marker box in the lane: its frame and x, lane row, the markers it stands for (the
/// first is drawn), and how many of those were hidden because they sat too close.
struct MBox {
    frame: u64,
    x: f32,
    row: usize,
    ixs: Vec<usize>,
    hidden: usize,
}

/// A marker label cut to `LABEL_CHARS`.
fn short_label(t: &str) -> String {
    if t.chars().count() > LABEL_CHARS {
        t.chars().take(LABEL_CHARS - 1).collect::<String>() + "…"
    } else {
        t.to_string()
    }
}

fn tick_step(px_per_frame: f32) -> u64 {
    for s in [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200, 18000, 36000] {
        if s as f32 * px_per_frame >= 64.0 {
            return s;
        }
    }
    72000
}

#[derive(Clone, Copy)]
enum Item {
    Header(usize),
    Row(usize, usize),
}

impl RollState {
    /// The selected notes with their spans, in the segment the roll last drew.
    pub fn selected_notes(&self) -> Vec<(InputKey, (u64, u64))> {
        let Some(m) = &self.model else { return vec![] };
        self.sel
            .iter()
            .filter_map(|(key, start)| {
                let (gi, ri) = row_of(m, *key)?;
                m.groups[gi].rows[ri].spans.iter().find(|sp| sp.0 == *start).map(|sp| (*key, *sp))
            })
            .collect()
    }

    /// Selects every press on every row of the segment the roll last drew. Returns how
    /// many presses are selected.
    pub fn select_all(&mut self) -> usize {
        let Some(m) = &self.model else { return 0 };
        self.sel = m.groups.iter().flat_map(|g| g.rows.iter().flat_map(|row| row.spans.iter().map(move |sp| (row.key, sp.0)))).collect();
        self.sel.len()
    }

    /// Replaces the selection (notes named by button and first frame).
    pub fn set_selection(&mut self, notes: impl IntoIterator<Item = (InputKey, u64)>) {
        self.sel = notes.into_iter().collect();
    }

    pub fn invalidate(&mut self) {
        self.gen += 1;
    }

    pub fn forget(&mut self) {
        self.model = None;
        self.open.clear();
        self.scroll_y = 0.0;
        self.invalidate();
    }

    pub fn zoom(&mut self, factor: f32) {
        self.px_per_frame = (self.px_per_frame * factor).clamp(0.02, 48.0);
    }

    /// Makes sure `frame` is in view on the next draw.
    pub fn follow_to(&mut self, frame: u64) {
        self.jump_to = Some(frame);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, col: &Collection, id: &str, playhead: Option<u64>, playing: bool, nb: &Neighbours) -> Option<Action> {
        let side_id = |s: &Option<Side>| s.as_ref().map(|s| s.id.clone());
        let stale = match &self.model {
            Some(m) => m.id != id || m.gen != self.gen || m.all_buttons != self.all_buttons || side_id(&m.prev) != nb.prev || side_id(&m.next) != nb.next,
            None => true,
        };
        if stale {
            if let Some(old) = self.model.as_ref().filter(|m| m.id != id) {
                // Crossing into a neighbour keeps the view where it was, so the picture doesn't
                // jump: the old segment's end is the new one's frame 0, and the other way round.
                self.scroll_x = if old.next.as_ref().is_some_and(|n| n.id == id) {
                    self.scroll_x - old.frames as f32
                } else if old.prev.as_ref().is_some_and(|p| p.id == id) {
                    self.scroll_x + old.prev.as_ref().map_or(0, |p| p.frames) as f32
                } else {
                    0.0
                };
                self.drag = None;
                self.gesture = None;
                self.sel.clear();
            }
            let mut m = build_model(col, id, self.gen, self.all_buttons);
            m.prev = nb.prev.as_deref().and_then(|p| build_side(col, p, &m.groups));
            m.next = nb.next.as_deref().and_then(|n| build_side(col, n, &m.groups));
            // Keep only selected presses that still exist after the edit.
            self.sel.retain(|(key, start)| row_of(&m, *key).is_some_and(|(g, r)| m.groups[g].rows[r].spans.iter().any(|s| s.0 == *start)));
            self.model = Some(m);
        }
        let model = self.model.take().unwrap();
        let action = self.draw(ui, &model, playhead, playing, nb.dirty.as_ref());
        self.model = Some(model);
        action
    }

    fn is_open(&self, g: &Group) -> bool {
        *self.open.get(&(g.port, g.device)).unwrap_or(&g.used())
    }

    fn draw(&mut self, ui: &mut egui::Ui, m: &Model, playhead: Option<u64>, playing: bool, dirty: Option<&DirtyBand>) -> Option<Action> {
        let mut action = None;
        // Toolbar.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Inputs").strong());
            if !self.sel.is_empty() {
                ui.label(egui::RichText::new(format!("{} selected", self.sel.len())).small());
            }
            ui.separator();
            let used: Vec<&str> = m.groups.iter().filter(|g| g.used()).map(|g| g.name.as_str()).collect();
            ui.label(egui::RichText::new(if used.is_empty() { "no inputs in this segment".to_string() } else { format!("uses {}", used.join(", ")) }).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.all_buttons, "all buttons");
                ui.checkbox(&mut self.labels, "labels");
                ui.checkbox(&mut self.follow, "follow");
                if ui.small_button("fit").on_hover_text("Fit the whole segment").clicked() {
                    let w = (ui.available_width() + 200.0 - LABEL_W).max(100.0);
                    self.px_per_frame = (w / m.frames.max(1) as f32).clamp(0.02, 48.0);
                    self.scroll_x = 0.0;
                }
                if ui.small_button("+").clicked() {
                    self.zoom(1.5);
                }
                if ui.small_button("−").clicked() {
                    self.zoom(1.0 / 1.5);
                }
                ui.label(egui::RichText::new(format!("{:.2} px/frame", self.px_per_frame)).weak().small());
            });
        });

        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let tl_left = rect.left() + LABEL_W;
        let tl_w = (rect.right() - tl_left).max(1.0);
        let font = FontId::proportional(11.0);
        let measure = |t: &str| ui.painter().layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x;
        // Expanded lane: each marker gets a row with room for its whole label, worked out in
        // pixels from the start of the segment, so rows don't change while scrolling.
        // Events take the top rows; RAM writes and everything else go below them.
        let mut exp_row = vec![0usize; m.markers.len()];
        let mut event_rows = 0;
        let mut lane_rows = 1;
        if self.markers_open && !m.markers.is_empty() {
            const MAX_ROWS: usize = 12;
            let mut base = 0;
            for events in [true, false] {
                let mut ends: Vec<f32> = vec![];
                for (i, mk) in m.markers.iter().enumerate().filter(|(_, mk)| (mk.glyph == "⚑") == events) {
                    let x = mk.frame as f32 * self.px_per_frame;
                    let label = short_label(&mk.label);
                    let want = if !self.labels || label.is_empty() { 10.0 } else { 14.0 + measure(&label) };
                    let r = match ends.iter().position(|e| *e <= x - 6.0) {
                        Some(r) => r,
                        None if ends.len() < MAX_ROWS => {
                            ends.push(f32::NEG_INFINITY);
                            ends.len() - 1
                        }
                        None => (0..ends.len()).min_by(|a, b| ends[*a].total_cmp(&ends[*b])).unwrap(),
                    };
                    ends[r] = x + want;
                    exp_row[i] = base + r;
                }
                if events {
                    event_rows = ends.len();
                }
                base += ends.len();
            }
            lane_rows = base.max(1);
        }
        let lane_h = MARK_H * lane_rows as f32;
        let rows_top = rect.top() + RULER_H + lane_h;
        let ppf = self.px_per_frame;

        // Layout of rows.
        let mut items: Vec<(f32, f32, Item)> = vec![];
        let mut y = 0.0;
        for (gi, g) in m.groups.iter().enumerate() {
            items.push((y, y + GROUP_H, Item::Header(gi)));
            y += GROUP_H;
            if self.is_open(g) {
                for ri in 0..g.rows.len() {
                    items.push((y, y + ROW_H, Item::Row(gi, ri)));
                    y += ROW_H;
                }
            }
        }
        let content_h = y;
        let view_h = (rect.bottom() - rows_top).max(1.0);

        // Scrolling and zoom.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if let Some(p) = pointer {
                if zoom != 1.0 && p.x > tl_left {
                    let anchor = self.scroll_x + (p.x - tl_left) / ppf;
                    self.zoom(zoom);
                    self.scroll_x = anchor - (p.x - tl_left) / self.px_per_frame;
                } else if p.x < tl_left {
                    self.scroll_y -= scroll.y;
                } else {
                    self.scroll_x -= (scroll.x + scroll.y) / ppf;
                }
            }
        }
        // While dragging in select mode, holding the pointer near either end of the
        // timeline scrolls it at a steady speed in pixels, so the pace follows the zoom.
        if self.gesture.is_some() {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let edge = 36.0;
                let (lo, hi) = (tl_left + edge, rect.right() - edge);
                let k = if p.x < lo { -((lo - p.x) / edge).min(1.5) } else if p.x > hi { ((p.x - hi) / edge).min(1.5) } else { 0.0 };
                if k != 0.0 {
                    let dt = ui.input(|i| i.stable_dt).min(0.05);
                    self.scroll_x += k * 500.0 * dt / self.px_per_frame;
                    ui.ctx().request_repaint();
                }
            }
        }
        let ppf = self.px_per_frame;
        let view_frames = tl_w / ppf;
        if let Some(f) = self.jump_to.take() {
            let f = f as f32;
            if f < self.scroll_x || f > self.scroll_x + view_frames {
                self.scroll_x = f - view_frames * 0.1;
            }
        }
        if let (Some(ph), true, true) = (playhead, playing, self.follow) {
            let ph = ph as f32;
            if ph < self.scroll_x || ph > self.scroll_x + view_frames * 0.9 {
                self.scroll_x = ph - view_frames * 0.1;
            }
        }
        // The view may run on into the neighbours, but no further.
        let lo = -view_frames * 0.05 - m.prev.as_ref().map_or(0, |p| p.frames) as f32;
        let hi = (m.frames + m.next.as_ref().map_or(0, |n| n.frames)) as f32 - view_frames * 0.5;
        self.scroll_x = self.scroll_x.clamp(lo, hi.max(0.0));
        self.scroll_y = self.scroll_y.clamp(0.0, (content_h - view_h).max(0.0));

        let x_of = |f: f32| tl_left + (f - self.scroll_x) * ppf;
        let frame_at = |x: f32| ((x - tl_left) / ppf + self.scroll_x).floor().max(0.0) as u64;
        // Outside the segment: the neighbours, which nothing on the roll may change. A few
        // pixels of slack keep the outer edges of the first and last presses grabbable.
        let seg_x = (x_of(0.0), x_of(m.frames as f32));
        let outside = |x: f32| x < seg_x.0 - 4.0 || x > seg_x.1 + 4.0;
        let f0 = self.scroll_x.floor().max(0.0) as u64;
        let f1 = ((self.scroll_x + view_frames).ceil().max(0.0) as u64 + 1).min(m.frames);

        // Marker boxes. Collapsed: one row; markers on one frame share a box, and a box too
        // close to the one before is hidden behind whichever matters more (events first),
        // which then shows "+N". Expanded: one box per marker, in the rows worked out above.
        // Either way a label runs until the next box in its row.
        let my = rect.top() + RULER_H + MARK_H * 0.5;
        let row_y = |r: usize| rect.top() + RULER_H + MARK_H * (r as f32 + 0.5);
        let prio = |mk: &Marker| match mk.glyph {
            "⚑" => 3,
            "✔" => 0,
            "▣" => 1,
            _ => 2,
        };
        let mut boxes: Vec<MBox> = vec![];
        let a0 = m.markers.partition_point(|mk| mk.frame < f0.saturating_sub(1));
        for (i, mk) in m.markers.iter().enumerate().skip(a0) {
            if mk.frame > f1 {
                break;
            }
            let x = x_of(mk.frame as f32 + 0.5 * (ppf >= 4.0) as u8 as f32).max(tl_left + 6.0);
            if self.markers_open {
                boxes.push(MBox { frame: mk.frame, x, row: exp_row[i], ixs: vec![i], hidden: 0 });
                continue;
            }
            // Room for a glyph and "+N", so a hidden marker always shows up in a count.
            const GAP: f32 = 30.0;
            match boxes.last_mut() {
                Some(b) if b.frame == mk.frame || x - b.x < 3.0 => b.ixs.push(i),
                Some(b) if x - b.x < GAP => {
                    let best = |b: &MBox| b.ixs.iter().map(|j| prio(&m.markers[*j])).max().unwrap_or(0);
                    if prio(mk) > best(b) {
                        // The new marker matters more: it takes the place of the last box.
                        let n = b.ixs.len() + b.hidden;
                        let mut ixs = std::mem::take(&mut b.ixs);
                        ixs.insert(0, i);
                        *b = MBox { frame: mk.frame, x, row: 0, ixs, hidden: n };
                    } else {
                        b.ixs.push(i);
                        b.hidden += 1;
                    }
                }
                _ => boxes.push(MBox { frame: mk.frame, x, row: 0, ixs: vec![i], hidden: 0 }),
            }
        }
        // Label text and colour for each box: the most telling marker's label, then "+N".
        let texts: Vec<(String, String, Color32)> = boxes
            .iter()
            .map(|b| {
                let mut named: Vec<&Marker> = b.ixs.iter().map(|i| &m.markers[*i]).collect();
                named.sort_by_key(|mk| (std::cmp::Reverse(prio(mk)), mk.label.is_empty()));
                let lead = named[0];
                // Markers on the box's own frame beyond the first, plus hidden neighbours.
                let extra = if self.markers_open { 0 } else { b.ixs.len() - 1 };
                let more = if extra > 0 { format!("+{extra}") } else { String::new() };
                let text = if self.labels { short_label(&lead.label) } else { String::new() };
                (more, text, lead.color)
            })
            .collect();
        let mut marker_rects: Vec<(Rect, usize)> = vec![];
        let mut marker_labels: Vec<(f32, f32, String, Color32)> = vec![];
        for (bi, b) in boxes.iter().enumerate() {
            let x = &b.x;
            let r = b.row;
            let next_x = boxes.iter().skip(bi + 1).find(|o| o.row == r).map_or(rect.right(), |o| o.x);
            let mut right = *x + 7.0;
            let (more, mut text, c) = texts[bi].clone();
            // The "+N" count stays whole; only the label is cut short.
            let pre = if more.is_empty() { 0.0 } else { measure(&format!("{more} ")) };
            if !more.is_empty() && pre - 3.0 <= next_x - x - 12.0 {
                marker_labels.push((*x + 8.0, row_y(r), more.clone(), Color32::from_gray(190)));
                right = *x + 8.0 + pre;
            }
            let avail = next_x - x - 14.0 - pre;
            if !text.is_empty() && avail > 14.0 {
                let mut full = measure(&text);
                while full > avail && text.chars().count() > 1 {
                    let n = text.trim_end_matches('…').chars().count();
                    let keep = ((n as f32 * avail / full) as usize).min(n - 1).max(1);
                    text = text.chars().take(keep).collect::<String>().trim_end().to_string() + "…";
                    full = measure(&text);
                    if keep == 1 {
                        break;
                    }
                }
                if full <= avail {
                    marker_labels.push((*x + 8.0 + pre, row_y(r), text, c));
                    right = *x + 10.0 + pre + full;
                }
            }
            let top = rect.top() + RULER_H + MARK_H * r as f32;
            marker_rects.push((Rect::from_min_max(Pos2::new(*x - 6.0, top), Pos2::new(right, top + MARK_H)), bi));
        }
        let marker_at = |p: Pos2| marker_rects.iter().find(|(r, _)| r.contains(p)).map(|(_, bi)| *bi);

        // Pointer interaction.
        let (pressed, down, released) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released()));
        let hit = |p: Pos2| -> Option<usize> {
            if p.y < rows_top {
                return None;
            }
            let yy = p.y - rows_top + self.scroll_y;
            items.iter().position(|(a, b, _)| yy >= *a && yy < *b)
        };
        // ---- select mode ----
        let ff_at = |x: f32| (x - tl_left) / ppf + self.scroll_x;
        let mods = ui.input(|i| i.modifiers);
        let op = if mods.shift && mods.alt {
            SelOp::Remove
        } else if mods.shift {
            SelOp::Add
        } else {
            SelOp::Replace
        };
        // Edges can be dragged only while at most one note is selected.
        let edges_ok = self.sel.len() <= 1;
        // The press under `p`, and whether `p` is on its left (true) or right (false) edge.
        let press_at = |p: Pos2| -> Option<(InputKey, (u64, u64), Option<bool>)> {
            let ix = hit(p)?;
            let Item::Row(gi, ri) = items[ix].2 else { return None };
            if p.x < tl_left {
                return None;
            }
            if outside(p.x) {
                return None;
            }
            let row = &m.groups[gi].rows[ri];
            let f = ff_at(p.x);
            let lo = (f - 8.0 / ppf).floor().max(0.0) as u64;
            let hi = (f + 8.0 / ppf).ceil().max(0.0) as u64 + 1;
            let mut body = None;
            let edges = edges_ok;
            for &sp in visible(&row.spans, lo, hi) {
                let (x0, x1) = (x_of(sp.0 as f32), x_of(sp.1 as f32).max(x_of(sp.0 as f32) + 1.5));
                // Wide notes have a thin grab zone on each edge (mostly outside the note, so a
                // drag on the note moves it); narrow ones have it just outside.
                let wide = x1 - x0 >= 16.0;
                if edges && ((wide && p.x >= x0 - 3.0 && p.x <= x0 + 1.5) || (!wide && p.x < x0 && p.x >= x0 - 4.0)) {
                    return Some((row.key, sp, Some(true)));
                }
                if edges && ((wide && p.x >= x1 - 1.5 && p.x <= x1 + 3.0) || (!wide && p.x > x1 && p.x <= x1 + 4.0)) {
                    return Some((row.key, sp, Some(false)));
                }
                if p.x >= x0 && p.x < x1 {
                    body = Some((row.key, sp, None));
                }
            }
            body
        };
        // Selected presses with their current spans.
        let sel_spans = |sel: &BTreeSet<PressId>| -> Vec<(InputKey, (u64, u64))> {
            sel.iter()
                .filter_map(|(key, start)| {
                    let (gi, ri) = row_of(m, *key)?;
                    m.groups[gi].rows[ri].spans.iter().find(|sp| sp.0 == *start).map(|sp| (*key, *sp))
                })
                .collect()
        };
        // Presses inside a marquee given as (frame, content y) corners.
        let in_area = |a: (f32, f32), b: (f32, f32)| -> Vec<PressId> {
            let (fa, fb) = (a.0.min(b.0), a.0.max(b.0));
            let (ya, yb) = (a.1.min(b.1), a.1.max(b.1));
            let mut out = vec![];
            for (y0, y1, item) in &items {
                if let Item::Row(gi, ri) = *item {
                    if *y1 > ya && *y0 < yb {
                        let row = &m.groups[gi].rows[ri];
                        for sp in &row.spans {
                            if (sp.0 as f32) < fb && (sp.1 as f32) > fa {
                                out.push((row.key, sp.0));
                            }
                        }
                    }
                }
            }
            out
        };
        let header_at = |p: Pos2| hit(p).is_some_and(|ix| matches!(items[ix].2, Item::Header(_)));
        if pressed && resp.hovered() {
            if let Some(p) = pointer {
                if p.x < tl_left && p.y >= rect.top() + RULER_H && p.y < rect.top() + RULER_H + MARK_H {
                    self.markers_open = !self.markers_open;
                } else if let Some(bi) = marker_at(p) {
                    action = Some(Action::OpenFrame(boxes[bi].frame));
                } else if p.x >= tl_left && outside(p.x) {
                    // In a neighbour: read-only, a click does nothing.
                } else if p.y < rect.top() + RULER_H && p.x >= tl_left {
                    // Only the frame ruler moves the playhead.
                    self.scrubbing = true;
                } else if p.y < rows_top && p.x >= tl_left {
                    // The marker lane, between markers: nothing.
                } else if p.x >= tl_left && !header_at(p) && hit(p).is_some() && mods.command && press_at(p).is_some() {
                    // Ctrl/Cmd+drag from a press: select an area.
                    let a = (ff_at(p.x), p.y - rows_top + self.scroll_y);
                    self.gesture = Some(Gesture::Marquee { a, b: a, op, cell: None });
                } else if p.x >= tl_left && !header_at(p) && hit(p).is_some() {
                    let row_here = match hit(p).map(|ix| items[ix].2) {
                        Some(Item::Row(gi, ri)) => Some((gi, ri)),
                        _ => None,
                    };
                    match press_at(p) {
                        Some((key, sp, Some(left))) => {
                            self.gesture = Some(Gesture::Resize { key, span: sp, left, to: if left { sp.0 } else { sp.1 } });
                        }
                        Some((key, sp, None)) => {
                            let id = (key, sp.0);
                            let anchor = ff_at(p.x);
                            let from = row_here.unwrap_or((0, 0));
                            if mods.shift {
                                // Shift+click adds or removes this press.
                                if !self.sel.remove(&id) {
                                    self.sel.insert(id);
                                    self.gesture = Some(Gesture::Move { anchor, delta: 0, click: None, from, dr: 0 });
                                }
                            } else {
                                // Pressing on an already selected press keeps the selection,
                                // so a group can be dragged; a plain click then selects just it.
                                let was = self.sel.contains(&id);
                                if !was {
                                    self.sel.clear();
                                    self.sel.insert(id);
                                }
                                self.gesture = Some(Gesture::Move { anchor, delta: 0, click: was.then_some(id), from, dr: 0 });
                            }
                        }
                        None => {
                            // An empty cell: a drag selects an area, or with drawing on (Edit >
                            // Drag to draw inputs) draws a press as long as the drag (a click
                            // makes one as long as the last). Ctrl/Cmd swaps the two; Shift or
                            // Alt always select.
                            let f = frame_at(p.x);
                            let cell = row_here.filter(|_| f < m.frames && ff_at(p.x) >= 0.0).map(|(gi, ri)| (gi, ri, f));
                            let draw = self.draw != mods.command && !mods.shift && !mods.alt;
                            match cell.filter(|_| draw) {
                                Some((group, row, f)) => {
                                    self.sel.clear();
                                    self.drag = Some(Drag { group, row, anchor: f, current: f, moved: false });
                                }
                                None => {
                                    let a = (ff_at(p.x), p.y - rows_top + self.scroll_y);
                                    self.gesture = Some(Gesture::Marquee { a, b: a, op, cell });
                                }
                            }
                        }
                    }
                } else if let Some(ix) = hit(p) {
                    match items[ix].2 {
                        Item::Header(gi) => {
                            let g = &m.groups[gi];
                            if p.x < tl_left {
                                let now = self.is_open(g);
                                self.open.insert((g.port, g.device), !now);
                            } else {
                                // A group header in the timeline: a drag selects an area,
                                // like an empty cell (a click clears the selection).
                                let a = (ff_at(p.x), p.y - rows_top + self.scroll_y);
                                self.gesture = Some(Gesture::Marquee { a, b: a, op, cell: None });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if let Some(p) = pointer.or_else(|| ui.input(|i| i.pointer.interact_pos())) {
            if self.scrubbing {
                let f = frame_at(p.x).min(m.frames.saturating_sub(1));
                if playhead != Some(f) {
                    action = Some(Action::Seek(f));
                }
            }
            if let Some(d) = &mut self.drag {
                d.current = frame_at(p.x).min(m.frames.saturating_sub(1));
                if d.current != d.anchor {
                    d.moved = true;
                }
            }
        }
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            match &mut self.gesture {
                Some(Gesture::Move { anchor, delta, from, dr, .. }) => {
                    let spans = sel_spans(&self.sel);
                    let lo = -(spans.iter().map(|(_, sp)| sp.0).min().unwrap_or(0) as i64);
                    let hi = m.frames as i64 - spans.iter().map(|(_, sp)| sp.1).max().unwrap_or(0) as i64;
                    *delta = ((ff_at(p.x) - *anchor).round() as i64).clamp(lo, hi.max(lo));
                    // Alt: transpose to the row under the pointer, within the same controller,
                    // as far as every selected press can go. Letting go of Alt first keeps the
                    // rows chosen; moving back to the first row with Alt held undoes them.
                    if mods.alt {
                        if let Some(Item::Row(g2, r2)) = hit(p).map(|ix| items[ix].2) {
                            if g2 == from.0 {
                                *dr = clamp_rows(m, &self.sel, r2 as i64 - from.1 as i64);
                            }
                        }
                    }
                }
                Some(Gesture::Resize { span, left, to, .. }) => {
                    let f = ff_at(p.x).round().max(0.0) as u64;
                    *to = if *left { f.min(span.1 - 1) } else { f.clamp(span.0 + 1, m.frames.max(span.0 + 1)) };
                }
                Some(Gesture::Marquee { b, .. }) => {
                    *b = (ff_at(p.x), p.y - rows_top + self.scroll_y);
                }
                None => {}
            }
        }
        if released || !down {
            if let Some(g) = self.gesture.take() {
                match g {
                    Gesture::Move { delta, click, dr, .. } => {
                        if delta == 0 && dr == 0 {
                            if let Some(c) = click {
                                self.sel.clear();
                                self.sel.insert(c);
                            }
                        } else if let Some((edits, from, new_sel)) = move_presses(m, &self.sel, delta, dr) {
                            self.sel = new_sel;
                            action = Some(Action::Edit { edits, from });
                        }
                    }
                    Gesture::Resize { key, span, left, to } => {
                        let new = if left { (to, span.1) } else { (span.0, to) };
                        if let (true, Some((gi, ri))) = (new != span, row_of(m, key)) {
                            let mut spans: Vec<(u64, u64)> = m.groups[gi].rows[ri].spans.iter().copied().filter(|sp| *sp != span).collect();
                            spans.push(new);
                            let spans = merge(spans);
                            let from = if left { span.0.min(to) } else { span.1.min(to) };
                            self.last_len = new.1 - new.0;
                            self.sel.remove(&(key, span.0));
                            if let Some(t) = span_at(&spans, new.0) {
                                self.sel.insert((key, t.0));
                            }
                            action = Some(Action::Edit { edits: vec![(key, spans)], from });
                        }
                    }
                    Gesture::Marquee { a, b, op, cell } => {
                        let tiny = ((a.0 - b.0) * ppf).abs() < 3.0 && (a.1 - b.1).abs() < 3.0;
                        let found = in_area(a, b);
                        if let (true, Some((gi, ri, f))) = (tiny, cell) {
                            let row = &m.groups[gi].rows[ri];
                            let span = (f, (f + self.last_len.max(1)).min(m.frames.max(f + 1)));
                            let mut spans = row.spans.clone();
                            spans.push(span);
                            let spans = merge(spans);
                            self.sel.clear();
                            if let Some(t) = span_at(&spans, f) {
                                self.sel.insert((row.key, t.0));
                            }
                            action = Some(Action::Edit { edits: vec![(row.key, spans)], from: f });
                        } else {
                            match op {
                                SelOp::Replace => self.sel = if tiny { BTreeSet::new() } else { found.into_iter().collect() },
                                SelOp::Add => self.sel.extend(found),
                                SelOp::Remove => {
                                    for id in found {
                                        self.sel.remove(&id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            self.scrubbing = false;
            if let Some(d) = self.drag.take() {
                let (a, b) = d.span(self.last_len, m.frames);
                let row = &m.groups[d.group].rows[d.row];
                let mut spans = row.spans.clone();
                spans.push((a, b));
                let spans = merge(spans);
                if d.moved {
                    self.last_len = b - a;
                }
                self.sel.clear();
                if let Some(t) = span_at(&spans, a) {
                    self.sel.insert((row.key, t.0));
                }
                action = Some(Action::Edit { edits: vec![(row.key, spans)], from: a });
            }
        }

        // Right-click removes the press under the pointer.
        if resp.hovered() && ui.input(|i| i.pointer.secondary_pressed()) && self.gesture.is_none() && self.drag.is_none() {
            if let Some((key, sp, _)) = pointer.and_then(press_at) {
                if let Some((gi, ri)) = row_of(m, key) {
                    let spans = m.groups[gi].rows[ri].spans.iter().copied().filter(|s| *s != sp).collect();
                    self.sel.remove(&(key, sp.0));
                    action = Some(Action::Edit { edits: vec![(key, spans)], from: sp.0 });
                }
            }
        }
        // Delete removes the selected presses; Escape clears the selection; Alt+Up/Down
        // moves them to the next button row.
        if !self.sel.is_empty() && !self.recording && !ui.ctx().egui_wants_keyboard_input() {
            let (del, esc, up, down_k) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                    i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown),
                )
            });
            let want = if up { -1 } else if down_k { 1 } else { 0 };
            if want != 0 {
                let dr = clamp_rows(m, &self.sel, want);
                if dr != 0 {
                    if let Some((edits, from, new_sel)) = move_presses(m, &self.sel, 0, dr) {
                        self.sel = new_sel;
                        action = Some(Action::Edit { edits, from });
                    }
                }
            }
            if del {
                let gone = sel_spans(&self.sel);
                let keys: BTreeSet<InputKey> = gone.iter().map(|(k, _)| *k).collect();
                let mut edits = vec![];
                for key in keys {
                    if let Some((gi, ri)) = row_of(m, key) {
                        let spans = m.groups[gi].rows[ri].spans.iter().copied().filter(|sp| !gone.contains(&(key, *sp))).collect();
                        edits.push((key, spans));
                    }
                }
                if let Some(from) = gone.iter().map(|(_, sp)| sp.0).min() {
                    action = Some(Action::Edit { edits, from });
                }
                self.sel.clear();
            } else if esc {
                self.sel.clear();
            }
        }
        // What would be selected if the drag ended now, for highlighting.
        let shown_sel: BTreeSet<PressId> = match &self.gesture {
            Some(Gesture::Marquee { a, b, op, .. }) => {
                let found = in_area(*a, *b);
                let mut s = if *op == SelOp::Replace { BTreeSet::new() } else { self.sel.clone() };
                if *op == SelOp::Remove {
                    for id in found {
                        s.remove(&id);
                    }
                } else {
                    s.extend(found);
                }
                s
            }
            _ => self.sel.clone(),
        };
        let (move_delta, move_dr) = match &self.gesture {
            Some(Gesture::Move { delta, dr, .. }) => (*delta, *dr),
            _ => (0, 0),
        };

        // ---- drawing ----
        let bg = Color32::from_gray(22);
        painter.rect_filled(rect, 0.0, bg);
        let step = tick_step(ppf);
        let first_tick = (f0 / step) * step;

        // The neighbours: (side, frame offset of its frame 0 on this roll, its visible frames).
        let sides: Vec<(&Side, f32, u64, u64)> = [(m.prev.as_ref(), true), (m.next.as_ref(), false)]
            .into_iter()
            .filter_map(|(s, before)| {
                let s = s?;
                let off = if before { -(s.frames as f32) } else { m.frames as f32 };
                let a = (self.scroll_x - off).floor().max(0.0) as u64;
                let b = ((self.scroll_x + view_frames - off).ceil().max(0.0) as u64 + 1).min(s.frames);
                (a < b).then_some((s, off, a, b))
            })
            .collect();

        // Rows.
        let rows_clip = Rect::from_min_max(Pos2::new(rect.left(), rows_top), rect.max);
        let rp = painter.with_clip_rect(rows_clip);
        let end_x = x_of(m.frames as f32).min(rect.right());
        for (y0, y1, item) in &items {
            let top = rows_top + y0 - self.scroll_y;
            let bot = rows_top + y1 - self.scroll_y;
            if bot < rows_top || top > rect.bottom() {
                continue;
            }
            let row_rect = Rect::from_min_max(Pos2::new(tl_left, top), Pos2::new(rect.right(), bot));
            match *item {
                Item::Header(gi) => {
                    let g = &m.groups[gi];
                    let c = port_color(g.port, g.device);
                    let tint = if g.used() { c.gamma_multiply(0.18) } else { Color32::from_gray(30) };
                    rp.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), top), Pos2::new(rect.right(), bot)), 0.0, tint);
                    // The union of the group's presses, so a closed group still shows activity.
                    let col_u = if self.is_open(g) { c.gamma_multiply(0.35) } else { c.gamma_multiply(0.8) };
                    for &(a, b) in visible(&g.union, f0, f1) {
                        let x0 = x_of(a as f32);
                        let x1 = x_of(b as f32).max(x0 + 1.5);
                        rp.rect_filled(Rect::from_min_max(Pos2::new(x0, top + 6.0), Pos2::new(x1, bot - 6.0)), 1.0, col_u);
                    }
                    for (sd, off, a0, b0) in &sides {
                        for &(a, b) in visible(&sd.union[gi], *a0, *b0) {
                            let x0 = x_of(a as f32 + off);
                            let x1 = x_of(b as f32 + off).max(x0 + 1.5);
                            rp.rect_filled(Rect::from_min_max(Pos2::new(x0, top + 6.0), Pos2::new(x1, bot - 6.0)), 1.0, col_u.gamma_multiply(0.6));
                        }
                    }
                    let a = g.other.partition_point(|f| *f < f0);
                    let b = g.other.partition_point(|f| *f < f1);
                    for f in &g.other[a..b] {
                        let x = x_of(*f as f32);
                        rp.line_segment([Pos2::new(x, top + 3.0), Pos2::new(x, bot - 3.0)], Stroke::new(1.0, Color32::from_rgb(235, 200, 120)));
                    }
                    rp.line_segment([Pos2::new(rect.left(), bot - 0.5), Pos2::new(rect.right(), bot - 0.5)], Stroke::new(1.0, Color32::from_gray(45)));
                }
                Item::Row(gi, ri) => {
                    let g = &m.groups[gi];
                    let row = &g.rows[ri];
                    if ri % 2 == 1 {
                        rp.rect_filled(row_rect, 0.0, Color32::from_gray(26));
                    }
                    let c = port_color(g.port, g.device);
                    let span_rect_f = |a: f32, b: f32| {
                        let x0 = x_of(a);
                        Rect::from_min_max(Pos2::new(x0, top + 2.0), Pos2::new(x_of(b).max(x0 + 1.5), bot - 2.0))
                    };
                    let span_rect = |a: u64, b: u64| span_rect_f(a as f32, b as f32);
                    let resizing = match &self.gesture {
                        Some(Gesture::Resize { key, span, left, to }) if *key == row.key => Some((*span, if *left { (*to, span.1) } else { (span.0, *to) })),
                        _ => None,
                    };
                    let hl = Stroke::new(1.5, Color32::WHITE);
                    for (sd, off, a0, b0) in &sides {
                        for &(a, b) in visible(&sd.rows[gi][ri], *a0, *b0) {
                            rp.rect_filled(span_rect_f(a as f32 + off, b as f32 + off), 2.0, c.gamma_multiply(0.6));
                        }
                    }
                    for &(a, b) in visible(&row.spans, f0, f1) {
                        let selected = shown_sel.contains(&(row.key, a));
                        if let Some((old, new)) = resizing {
                            if old == (a, b) {
                                rp.rect_filled(span_rect(new.0, new.1), 2.0, c);
                                rp.rect_stroke(span_rect(new.0, new.1), 2.0, hl, StrokeKind::Inside);
                                continue;
                            }
                        }
                        if selected && (move_delta != 0 || move_dr != 0) {
                            // Where the press was; the moved copy is drawn below.
                            rp.rect_stroke(span_rect(a, b), 2.0, Stroke::new(1.0, c.gamma_multiply(0.6)), StrokeKind::Inside);
                            continue;
                        }
                        rp.rect_filled(span_rect(a, b), 2.0, if selected { c.linear_multiply(1.25) } else { c });
                        if selected {
                            rp.rect_stroke(span_rect(a, b), 2.0, hl, StrokeKind::Inside);
                        }
                    }
                    // Moved copies of the selected presses from the row `move_dr` above.
                    let src = ri as i64 - move_dr;
                    if (move_delta != 0 || move_dr != 0) && src >= 0 && (src as usize) < g.rows.len() {
                        let srow = &g.rows[src as usize];
                        for &(a, b) in &srow.spans {
                            if self.sel.contains(&(srow.key, a)) {
                                let (na, nb) = ((a as i64 + move_delta) as u64, (b as i64 + move_delta) as u64);
                                rp.rect_filled(span_rect(na, nb), 2.0, c);
                                rp.rect_stroke(span_rect(na, nb), 2.0, hl, StrokeKind::Inside);
                            }
                        }
                    }
                    if let Some(d) = &self.drag {
                        if d.group == gi && d.row == ri {
                            let (a, b) = d.span(self.last_len, m.frames);
                            let r = Rect::from_min_max(Pos2::new(x_of(a as f32), top + 1.0), Pos2::new(x_of(b as f32).max(x_of(a as f32) + 2.0), bot - 1.0));
                            rp.rect_filled(r, 2.0, Color32::WHITE.gamma_multiply(0.7));
                        }
                    }
                }
            }
        }
        // Grid lines, per-frame lines when zoomed in, and the area past the end.
        let grid = painter.with_clip_rect(Rect::from_min_max(Pos2::new(tl_left, rect.top()), rect.max));
        if ppf >= 6.0 {
            for f in f0..f1 {
                let x = x_of(f as f32);
                grid.line_segment([Pos2::new(x, rows_top), Pos2::new(x, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(6)));
            }
        }
        let mut t = first_tick;
        while t <= f1 {
            let x = x_of(t as f32);
            grid.line_segment([Pos2::new(x, rect.top() + RULER_H * 0.5), Pos2::new(x, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(14)));
            grid.text(Pos2::new(x + 3.0, rect.top() + 2.0), Align2::LEFT_TOP, t.to_string(), FontId::monospace(10.0), Color32::from_gray(150));
            t += step;
        }
        // The neighbours' own frame numbers, dimmer.
        for (_, off, a0, b0) in &sides {
            let mut t = (a0 / step) * step;
            while t < *b0 {
                // Frame 0 of the next segment is this one's end, already labelled.
                if t == 0 && *off > 0.0 {
                    t += step;
                    continue;
                }
                let x = x_of(t as f32 + off);
                grid.line_segment([Pos2::new(x, rect.top() + RULER_H * 0.5), Pos2::new(x, rect.top() + RULER_H)], Stroke::new(1.0, Color32::from_white_alpha(10)));
                grid.text(Pos2::new(x + 3.0, rect.top() + 2.0), Align2::LEFT_TOP, t.to_string(), FontId::monospace(10.0), Color32::from_gray(90));
                t += step;
            }
        }
        // Frames whose hashes aren't known: a band on the ruler in the tree dot's colour.
        let mut stale_rect = None;
        if let Some(u) = dirty.filter(|u| u.from < m.frames) {
            let (x0, x1) = (x_of(u.from as f32).max(tl_left), end_x);
            if x1 > x0 {
                let r = Rect::from_min_max(Pos2::new(x0, rect.top() + RULER_H - 5.0), Pos2::new(x1, rect.top() + RULER_H));
                grid.rect_filled(r, 0.0, u.color.gamma_multiply(0.7));
                stale_rect = Some((Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.top() + RULER_H)), u.hint.clone()));
            }
        }
        // Outside the segment: dimmed, with the boundary marked.
        let start_x = x_of(0.0);
        if start_x > tl_left {
            grid.rect_filled(Rect::from_min_max(Pos2::new(tl_left, rect.top()), Pos2::new(start_x, rect.bottom())), 0.0, Color32::from_black_alpha(110));
            grid.line_segment([Pos2::new(start_x, rect.top()), Pos2::new(start_x, rect.bottom())], Stroke::new(1.0, Color32::from_gray(90)));
        }
        if end_x < rect.right() {
            grid.rect_filled(Rect::from_min_max(Pos2::new(end_x, rect.top()), rect.max), 0.0, Color32::from_black_alpha(110));
            grid.line_segment([Pos2::new(end_x, rect.top()), Pos2::new(end_x, rect.bottom())], Stroke::new(1.0, Color32::from_gray(90)));
        }
        // The neighbours' titles in the marker lane, against the boundary.
        let title_font = FontId::proportional(11.0);
        if let Some(p) = &m.prev {
            if start_x > tl_left + 40.0 {
                let t = format!("⏴ {}", short_label(&p.title));
                grid.text(Pos2::new(start_x - 6.0, my), Align2::RIGHT_CENTER, t, title_font.clone(), Color32::from_gray(150));
            }
        }
        if let Some(n) = &m.next {
            if end_x < rect.right() - 40.0 {
                let t = format!("{} ⏵", short_label(&n.title));
                grid.text(Pos2::new(end_x + 6.0, my), Align2::LEFT_CENTER, t, title_font, Color32::from_gray(150));
            }
        }
        if self.recording {
            grid.rect_filled(Rect::from_min_max(Pos2::new(tl_left, rect.top()), Pos2::new(rect.right(), rect.top() + RULER_H)), 0.0, Color32::from_rgba_unmultiplied(210, 40, 40, 70));
        }
        grid.line_segment([Pos2::new(tl_left, rect.top() + RULER_H), Pos2::new(rect.right(), rect.top() + RULER_H)], Stroke::new(1.0, Color32::from_gray(50)));
        grid.line_segment([Pos2::new(tl_left, rows_top), Pos2::new(rect.right(), rows_top)], Stroke::new(1.0, Color32::from_gray(50)));

        // Markers lane.
        let mut hover_text: Option<String> = None;
        for b in &boxes {
            let first = b.ixs.iter().map(|i| &m.markers[*i]).max_by_key(|mk| prio(mk)).unwrap();
            let x = &b.x;
            let y = row_y(b.row);
            grid.line_segment([Pos2::new(*x, y + MARK_H * 0.5), Pos2::new(*x, rect.bottom())], Stroke::new(1.0, first.color.gamma_multiply(0.35)));
            grid.text(Pos2::new(*x, y), Align2::CENTER_CENTER, first.glyph, FontId::proportional(12.0), first.color);
        }
        for (x, y, text, c) in &marker_labels {
            grid.text(Pos2::new(*x, *y), Align2::LEFT_CENTER, text, font.clone(), *c);
        }
        if let Some(bi) = pointer.and_then(marker_at) {
            let (f, ixs) = (&boxes[bi].frame, &boxes[bi].ixs);
            let lines: Vec<String> = ixs.iter().map(|i| format!("frame {}: {}", m.markers[*i].frame, m.markers[*i].text)).collect();
            hover_text = Some(format!("{}\nclick to edit frame {f}", lines.join("\n")));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // Playhead and hover line.
        if let Some(ph) = playhead {
            let x0 = x_of(ph as f32);
            let x1 = x_of(ph as f32 + 1.0);
            let hl = Color32::from_rgb(255, 230, 120);
            if x1 - x0 >= 3.0 {
                grid.rect_filled(Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.bottom())), 0.0, hl.gamma_multiply(0.12));
            }
            let x = (x0 + x1) * 0.5;
            grid.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.5, hl));
            grid.add(egui::Shape::convex_polygon(vec![Pos2::new(x - 5.0, rect.top()), Pos2::new(x + 5.0, rect.top()), Pos2::new(x, rect.top() + 7.0)], hl, Stroke::NONE));
        }
        if let Some(p) = pointer {
            if rect.contains(p) && p.x >= tl_left && hover_text.is_none() {
                let ff = ff_at(p.x);
                let side = if ff < 0.0 { m.prev.as_ref().map(|s| (s, ff + s.frames as f32)) } else if ff >= m.frames as f32 { m.next.as_ref().map(|s| (s, ff - m.frames as f32)) } else { None };
                if let Some((s, f)) = side.filter(|(s, f)| *f >= 0.0 && *f < s.frames as f32) {
                    hover_text = Some(format!("\"{}\", frame {} (step or play across to edit it)", s.title, f.floor() as u64));
                } else if let Some((_, hint)) = stale_rect.filter(|(r, _)| r.contains(p)) {
                    hover_text = Some(hint);
                }
            }
            if rect.contains(p) && p.x >= tl_left && self.drag.is_none() && ff_at(p.x) >= 0.0 {
                let f = frame_at(p.x);
                if f < m.frames {
                    let x = x_of(f as f32 + 0.5);
                    grid.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, Color32::from_white_alpha(30)));
                    if hover_text.is_none() && p.y >= rows_top {
                        if let Some(ix) = hit(p) {
                            let what = match items[ix].2 {
                                Item::Header(gi) => m.groups[gi].name.clone(),
                                Item::Row(gi, ri) => format!("{} {}", m.groups[gi].name, m.groups[gi].rows[ri].label),
                            };
                            hover_text = Some(format!("{what} · frame {f}"));
                        }
                    }
                }
            }
        }
        if let Some(d) = &self.drag {
            let (a, b) = d.span(self.last_len, m.frames);
            hover_text = Some(format!("new press, frames {a}..={} ({} frames)", b - 1, b - a));
        }
        match &self.gesture {
            Some(Gesture::Marquee { a, b, op, .. }) => {
                let to_screen = |(f, y): (f32, f32)| Pos2::new(x_of(f), rows_top + y - self.scroll_y);
                let r = Rect::from_two_pos(to_screen(*a), to_screen(*b));
                let mc = if *op == SelOp::Remove { Color32::from_rgb(230, 90, 80) } else { Color32::from_rgb(120, 180, 255) };
                let mp = painter.with_clip_rect(Rect::from_min_max(Pos2::new(tl_left, rows_top), rect.max));
                mp.rect_filled(r, 0.0, mc.gamma_multiply(0.12));
                mp.rect_stroke(r, 0.0, Stroke::new(1.0, mc), StrokeKind::Inside);
            }
            Some(Gesture::Move { delta, dr, .. }) if *delta != 0 || *dr != 0 => {
                let rows = if *dr != 0 { format!(", {} row{} {}", dr.abs(), if dr.abs() == 1 { "" } else { "s" }, if *dr > 0 { "down" } else { "up" }) } else { String::new() };
                hover_text = Some(format!("move {} press{} by {delta:+} frames{rows}", self.sel.len(), if self.sel.len() == 1 { "" } else { "es" }))
            }
            Some(Gesture::Resize { span, left, to, .. }) => {
                let (a, b) = if *left { (*to, span.1) } else { (span.0, *to) };
                hover_text = Some(format!("press frames {a}..={} ({} frames)", b - 1, b - a));
            }
            _ => {}
        }

        // Label column, drawn last so it covers the timeline.
        let lp = painter.with_clip_rect(Rect::from_min_max(rect.min, Pos2::new(tl_left, rect.bottom())));
        lp.rect_filled(Rect::from_min_max(rect.min, Pos2::new(tl_left, rect.bottom())), 0.0, Color32::from_gray(28));
        lp.text(Pos2::new(rect.left() + 6.0, rect.top() + RULER_H * 0.5), Align2::LEFT_CENTER, "frame", FontId::proportional(11.0), Color32::from_gray(120));
        let lane_label = format!("{} markers", if self.markers_open { "⏷" } else { "⏵" });
        lp.text(Pos2::new(rect.left() + 10.0, my), Align2::LEFT_CENTER, lane_label, FontId::proportional(12.0), Color32::from_gray(200));
        let n_markers = m.markers.len();
        if n_markers > 0 {
            lp.text(Pos2::new(tl_left - 6.0, my), Align2::RIGHT_CENTER, n_markers.to_string(), FontId::monospace(10.5), Color32::from_gray(150));
        }
        if self.markers_open && event_rows > 0 && lane_rows > event_rows {
            lp.text(Pos2::new(rect.left() + 26.0, row_y(event_rows)), Align2::LEFT_CENTER, "writes, other", FontId::proportional(11.0), Color32::from_gray(120));
        }
        if self.markers_open {
            // Faint lines between lane rows, and a stronger one between events and the rest.
            for r in 1..lane_rows {
                let y = rect.top() + RULER_H + MARK_H * r as f32;
                let c = if r == event_rows { Color32::from_gray(55) } else { Color32::from_gray(36) };
                painter.line_segment([Pos2::new(tl_left, y), Pos2::new(rect.right(), y)], Stroke::new(1.0, c));
            }
        }
        let lrows = lp.with_clip_rect(Rect::from_min_max(Pos2::new(rect.left(), rows_top), Pos2::new(tl_left, rect.bottom())));
        for (y0, y1, item) in &items {
            let top = rows_top + y0 - self.scroll_y;
            let bot = rows_top + y1 - self.scroll_y;
            if bot < rows_top || top > rect.bottom() {
                continue;
            }
            let mid = (top + bot) * 0.5;
            match *item {
                Item::Header(gi) => {
                    let g = &m.groups[gi];
                    let c = port_color(g.port, g.device);
                    let open = self.is_open(g);
                    if g.used() {
                        lrows.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), top), Pos2::new(rect.left() + 4.0, bot)), 0.0, c);
                        lrows.rect_filled(Rect::from_min_max(Pos2::new(rect.left() + 4.0, top), Pos2::new(tl_left, bot)), 0.0, c.gamma_multiply(0.22));
                    }
                    let tc = if g.used() { Color32::from_gray(235) } else { Color32::from_gray(120) };
                    lrows.text(Pos2::new(rect.left() + 10.0, mid), Align2::LEFT_CENTER, format!("{} {}", if open { "⏷" } else { "⏵" }, g.name), FontId::proportional(12.5), tc);
                    let note = if g.presses > 0 { format!("{}", g.presses) } else if g.used() { "·".into() } else { "—".into() };
                    lrows.text(Pos2::new(tl_left - 6.0, mid), Align2::RIGHT_CENTER, note, FontId::monospace(10.5), if g.used() { c } else { Color32::from_gray(90) });
                }
                Item::Row(gi, ri) => {
                    let g = &m.groups[gi];
                    let row = &g.rows[ri];
                    let has = !row.spans.is_empty();
                    lrows.text(Pos2::new(rect.left() + 26.0, mid), Align2::LEFT_CENTER, &row.label, FontId::monospace(10.5), if has { Color32::from_gray(225) } else { Color32::from_gray(110) });
                    if has {
                        lrows.circle_filled(Pos2::new(rect.left() + 17.0, mid), 2.5, port_color(g.port, g.device));
                    }
                }
            }
        }
        lp.line_segment([Pos2::new(tl_left - 0.5, rect.top()), Pos2::new(tl_left - 0.5, rect.bottom())], Stroke::new(1.0, Color32::from_gray(55)));
        // Vertical scroll indicator.
        if content_h > view_h {
            let frac = view_h / content_h;
            let h = (view_h * frac).max(16.0);
            let y = rows_top + (self.scroll_y / (content_h - view_h)) * (view_h - h);
            painter.rect_filled(Rect::from_min_size(Pos2::new(tl_left - 4.0, y), egui::vec2(3.0, h)), 1.5, Color32::from_gray(90));
        }

        if let Some(t) = hover_text {
            resp.clone().on_hover_ui_at_pointer(|ui| {
                ui.label(t);
            });
        }
        let in_rows = |p: Pos2| p.y >= rows_top && p.x >= tl_left && !header_at(p) && hit(p).is_some();
        let icon = match &self.gesture {
            Some(Gesture::Move { dr, .. }) => Some(if mods.alt || *dr != 0 { egui::CursorIcon::AllScroll } else { egui::CursorIcon::Grabbing }),
            Some(Gesture::Resize { .. }) => Some(egui::CursorIcon::ResizeHorizontal),
            Some(Gesture::Marquee { .. }) => None,
            None if self.drag.is_some() => Some(egui::CursorIcon::Crosshair),
            None if resp.hovered() => match pointer {
                Some(p) if mods.command && in_rows(p) && press_at(p).is_some() => None,
                Some(p) => match press_at(p) {
                    Some((_, _, edge)) => Some(if edge.is_some() { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::Grab }),
                    None if in_rows(p) && self.draw != mods.command && !mods.shift && !mods.alt => Some(egui::CursorIcon::Crosshair),
                    None => None,
                },
                None => None,
            },
            None => None,
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
        action
    }
}

/// How many rows (toward `want`) every selected press can move and stay in its controller.
fn clamp_rows(m: &Model, sel: &BTreeSet<PressId>, want: i64) -> i64 {
    let mut dr = want;
    for (key, _) in sel {
        if let Some((gi, ri)) = row_of(m, *key) {
            let n = m.groups[gi].rows.len() as i64;
            dr = dr.clamp(-(ri as i64), n - 1 - ri as i64);
        }
    }
    if dr.signum() != want.signum() {
        0
    } else {
        dr
    }
}

/// The selected presses moved `delta` frames and `dr` rows down (same controller).
/// Returns the new spans for every touched button, the first changed frame, and the
/// moved presses' new ids.
fn move_presses(m: &Model, sel: &BTreeSet<PressId>, delta: i64, dr: i64) -> Option<(Vec<(InputKey, Vec<(u64, u64)>)>, u64, BTreeSet<PressId>)> {
    let moved: Vec<(InputKey, (u64, u64))> = sel
        .iter()
        .filter_map(|(key, start)| {
            let (gi, ri) = row_of(m, *key)?;
            m.groups[gi].rows[ri].spans.iter().find(|sp| sp.0 == *start).map(|sp| (*key, *sp))
        })
        .collect();
    if moved.is_empty() {
        return None;
    }
    let mut rows: HashMap<InputKey, Vec<(u64, u64)>> = HashMap::new();
    let spans_of = |key: InputKey| row_of(m, key).map(|(gi, ri)| m.groups[gi].rows[ri].spans.clone()).unwrap_or_default();
    // Take the moved presses out first, then put them in their new places.
    for (key, sp) in &moved {
        let v = rows.entry(*key).or_insert_with(|| spans_of(*key));
        v.retain(|s| s != sp);
    }
    let mut from = u64::MAX;
    let mut placed = vec![];
    for (key, sp) in &moved {
        let (gi, ri) = row_of(m, *key)?;
        let to_key = m.groups[gi].rows.get((ri as i64 + dr) as usize)?.key;
        let a = (sp.0 as i64 + delta).max(0) as u64;
        rows.entry(to_key).or_insert_with(|| spans_of(to_key)).push((a, a + (sp.1 - sp.0)));
        from = from.min(sp.0.min(a));
        placed.push((to_key, a));
    }
    let mut edits = vec![];
    let mut new_sel = BTreeSet::new();
    for (key, spans) in rows {
        let spans = merge(spans);
        for (k, a) in &placed {
            if *k == key {
                if let Some(t) = span_at(&spans, *a) {
                    new_sel.insert((key, t.0));
                }
            }
        }
        edits.push((key, spans));
    }
    Some((edits, from, new_sel))
}
