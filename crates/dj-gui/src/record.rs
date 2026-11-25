//! Recording inputs from the keyboard into the piano roll (docs/gui.md#recording):
//! key bindings, the Record dropdown's options, and writing a frame's input.

use crate::strings as t;
use anyhow::Result;
use dj_core::collection::{KeyboardLocal, RecordLocal};
use dj_core::controls::{control_from_name, control_name};
use dj_core::format::{direction, direction_of, is_binary, Command, InputKey, Segment, DEVICE_ANALOG, DEVICE_JOYPAD};
use egui::Key;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Joypad buttons in the order the mapping dialog lists them (libretro ids).
pub const BUTTONS: [u32; 16] = [4, 5, 6, 7, 8, 0, 9, 1, 2, 3, 10, 11, 12, 13, 14, 15];

/// The stick directions a key can play, after the buttons: the left stick's X- (left), X+,
/// Y- (up) and Y+, then the right stick's. A key pushes the stick all the way.
pub const STICKS: [u32; 8] = [16, 17, 18, 19, 20, 21, 22, 23];

/// The input a bindable control (a button id of `BUTTONS`, or one of `STICKS`) is on
/// controller `port`.
pub fn control_key(id: u32, port: u32) -> InputKey {
    if id < 16 {
        return InputKey { port, device: DEVICE_JOYPAD, index: 0, id };
    }
    let d = id - 16;
    direction(InputKey { port, device: DEVICE_ANALOG, index: d / 4, id: (d / 2) % 2 }, d % 2 == 1)
}

/// The inverse of `control_key`, whatever the port.
fn control_id(key: InputKey) -> Option<u32> {
    match direction_of(key) {
        Some((axis, positive)) => Some(16 + axis.index * 4 + axis.id * 2 + positive as u32),
        None => (key.device == DEVICE_JOYPAD && key.index == 0 && key.id < 16).then_some(key.id),
    }
}

/// Which key plays each button of the controller being recorded, plus the two
/// frame-by-frame keys.
#[derive(Clone, PartialEq, Debug)]
pub struct Bindings {
    /// Control to key: a joypad button id, or a stick direction (`STICKS`).
    pub buttons: BTreeMap<u32, Key>,
    pub commit: Option<Key>,
    pub rewind: Option<Key>,
}

impl Default for Bindings {
    /// WASD for the d-pad, J = B, K = A, U = Select, I = Start; Space commits and
    /// Backspace rewinds. (Per-console defaults are on the roadmap.)
    fn default() -> Self {
        let buttons = [(4, Key::W), (5, Key::S), (6, Key::A), (7, Key::D), (0, Key::J), (8, Key::K), (2, Key::U), (3, Key::I)].into_iter().collect();
        Bindings { buttons, commit: Some(Key::Space), rewind: Some(Key::Backspace) }
    }
}

/// A bindable control by its name in `[keyboard]` and `[[ports]]` (`B`, `L2`, `RIGHT Y-`).
fn button_id(name: &str) -> Option<u32> {
    control_from_name(0, name).and_then(control_id)
}

pub fn button_name(id: u32) -> String {
    control_name(control_key(id, 0)).unwrap_or_else(|| "?".into())
}

impl Bindings {
    /// From `[keyboard]` in local.toml; the defaults when there is none.
    pub fn from_local(k: Option<&KeyboardLocal>) -> Bindings {
        let Some(k) = k else { return Bindings::default() };
        let d = Bindings::default();
        let buttons = k.buttons.iter().filter_map(|(b, key)| Some((button_id(b)?, Key::from_name(key)?))).collect();
        let key = |v: &Option<String>, def: Option<Key>| match v {
            Some(s) if s.is_empty() => None,
            Some(s) => Key::from_name(s).or(def),
            None => def,
        };
        Bindings { buttons, commit: key(&k.commit, d.commit), rewind: key(&k.rewind, d.rewind) }
    }

    pub fn to_local(&self) -> KeyboardLocal {
        KeyboardLocal {
            buttons: self.buttons.iter().map(|(b, k)| (button_name(*b), k.name().to_string())).collect(),
            commit: Some(self.commit.map(|k| k.name().to_string()).unwrap_or_default()),
            rewind: Some(self.rewind.map(|k| k.name().to_string()).unwrap_or_default()),
        }
    }

    /// The buttons and stick directions of controller `port` whose keys are held.
    pub fn chord(&self, held: &HashSet<Key>, port: u32) -> Vec<InputKey> {
        self.buttons.iter().filter(|(_, k)| held.contains(k)).map(|(b, _)| control_key(*b, port)).collect()
    }

    /// The stick axes (stick, axis) that have a key on either direction: recording treats
    /// those as buttons.
    pub fn stick_axes(&self) -> BTreeSet<(u32, u32)> {
        self.buttons.keys().filter_map(|b| direction_of(control_key(*b, 0))).map(|(a, _)| (a.index, a.id)).collect()
    }

    /// True when `k` plays a button.
    pub fn is_button(&self, k: Key) -> bool {
        self.buttons.values().any(|b| *b == k)
    }
}

/// The options in the Record button's dropdown.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    /// Off: real time (Space is play/pause). On: Space commits one frame at a time.
    pub frame_by_frame: bool,
    /// Rewind clears the input on each frame it leaves (Shift+rewind does the opposite).
    pub rewind_clears: bool,
    /// Each recorded frame clears every controller's input first. Off: held buttons are
    /// only added, nothing is released.
    pub overwrite: bool,
    /// Recording past the last frame makes the segment longer. Off: it pauses there.
    pub extend: bool,
    /// The controller the bindings drive, 0-based.
    pub port: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { frame_by_frame: false, rewind_clears: false, overwrite: true, extend: false, port: 0 }
    }
}

impl Settings {
    pub fn from_local(r: Option<&RecordLocal>) -> Settings {
        let d = Settings::default();
        let Some(r) = r else { return d };
        Settings {
            frame_by_frame: r.frame_by_frame.unwrap_or(d.frame_by_frame),
            rewind_clears: r.rewind_clears.unwrap_or(d.rewind_clears),
            overwrite: r.overwrite.unwrap_or(d.overwrite),
            extend: r.extend.unwrap_or(d.extend),
            port: r.port.unwrap_or(d.port),
        }
    }

    pub fn to_local(&self) -> RecordLocal {
        RecordLocal { frame_by_frame: Some(self.frame_by_frame), rewind_clears: Some(self.rewind_clears), overwrite: Some(self.overwrite), extend: Some(self.extend), port: Some(self.port) }
    }
}

/// Every on/off input any line of `r` sets, with both directions of the stick axes in
/// `sticks` (stick, axis) that it sets.
fn binary_keys(r: &Segment, sticks: &BTreeSet<(u32, u32)>) -> BTreeSet<InputKey> {
    r.lines
        .iter()
        .flat_map(|l| match l.command {
            Command::Input { key, .. } if is_binary(key) => vec![key],
            Command::Input { key, .. } if key.device == DEVICE_ANALOG && sticks.contains(&(key.index, key.id)) => vec![direction(key, false), direction(key, true)],
            _ => vec![],
        })
        .collect()
}

/// Writes frame `f` of `r`: the buttons in `chord` are held there. With `overwrite`, every
/// other on/off input (on every controller) is released on that frame; without it, nothing
/// is released. `sticks` are the stick axes keys play (`Bindings::stick_axes`): a
/// direction in `chord` pushes its stick all the way, and overwriting lets those sticks go
/// like buttons. A stick no key plays keeps its values. Returns whether anything changed.
pub fn write_frame(r: &mut Segment, f: u64, chord: &[InputKey], overwrite: bool, sticks: &BTreeSet<(u32, u32)>) -> Result<bool> {
    let mut keys: BTreeSet<InputKey> = chord.iter().copied().collect();
    if overwrite {
        keys.extend(binary_keys(r, sticks));
    }
    let mut edits = vec![];
    for key in keys {
        let mut spans = r.binary_spans(key);
        let want = chord.contains(&key);
        let at = spans.iter().position(|(a, b)| *a <= f && f < *b);
        match (want, at) {
            (true, None) => {
                spans.push((f, f + 1));
                edits.push((key, spans));
            }
            (false, Some(i)) => {
                let (a, b) = spans.remove(i);
                spans.extend([(a, f), (f + 1, b)].into_iter().filter(|(x, y)| y > x));
                edits.push((key, spans));
            }
            _ => {}
        }
    }
    let changed = !edits.is_empty();
    r.set_binary_spans_many(&edits)?;
    Ok(changed)
}

/// Releases every on/off input on frame `f`, and the sticks keys play.
pub fn clear_frame(r: &mut Segment, f: u64, sticks: &BTreeSet<(u32, u32)>) -> Result<bool> {
    write_frame(r, f, &[], true, sticks)
}

/// A slot in the mapping dialog.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Slot {
    Button(u32),
    Commit,
    Rewind,
}

/// File > Keyboard mapping…: edits a copy of the bindings.
pub struct MappingDialog {
    pub draft: Bindings,
    /// The slot waiting for a key press.
    pub waiting: Option<Slot>,
}

pub enum MappingResult {
    Save(Bindings),
    Cancel,
}

impl MappingDialog {
    pub fn new(b: &Bindings) -> Self {
        MappingDialog { draft: b.clone(), waiting: None }
    }

    fn get(&self, s: Slot) -> Option<Key> {
        match s {
            Slot::Button(b) => self.draft.buttons.get(&b).copied(),
            Slot::Commit => self.draft.commit,
            Slot::Rewind => self.draft.rewind,
        }
    }

    /// Puts `key` on `slot`, taking it off any other slot that had it.
    pub fn assign(&mut self, slot: Slot, key: Option<Key>) {
        if let Some(k) = key {
            self.draft.buttons.retain(|_, v| *v != k);
            if self.draft.commit == Some(k) {
                self.draft.commit = None;
            }
            if self.draft.rewind == Some(k) {
                self.draft.rewind = None;
            }
        }
        match (slot, key) {
            (Slot::Button(b), Some(k)) => {
                self.draft.buttons.insert(b, k);
            }
            (Slot::Button(b), None) => {
                self.draft.buttons.remove(&b);
            }
            (Slot::Commit, k) => self.draft.commit = k,
            (Slot::Rewind, k) => self.draft.rewind = k,
        }
    }

    /// `controls` and `port`: what the buttons of the controller being recorded are called.
    pub fn show(&mut self, ctx: &egui::Context, controls: &dj_core::controls::Controls, port: u32) -> Option<MappingResult> {
        // A key pressed while a slot waits goes to that slot (Escape stops waiting).
        if let Some(slot) = self.waiting {
            let pressed = ctx.input_mut(|i| {
                let mut got = None;
                i.events.retain(|e| match e {
                    egui::Event::Key { key, pressed: true, repeat: false, .. } if got.is_none() => {
                        got = Some(*key);
                        false
                    }
                    _ => true,
                });
                got
            });
            match pressed {
                Some(Key::Escape) => self.waiting = None,
                Some(k) => {
                    self.assign(slot, Some(k));
                    self.waiting = None;
                }
                None => {}
            }
        }
        let mut out = None;
        let mut open = true;
        egui::Window::new(t::KEYBOARD_MAPPING_TITLE).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(egui::RichText::new(t::KEYS_FOR_THE_CONTROLLER_BEING).weak());
            ui.add_space(4.0);
            // The buttons the core or the project names, by those names; for a core that
            // names none, all of them. A button that has a key shows whatever else.
            let mut slots: Vec<(String, Slot)> = BUTTONS
                .iter()
                .filter_map(|b| {
                    let key = InputKey { port, device: DEVICE_JOYPAD, index: 0, id: *b };
                    let shown = self.get(Slot::Button(*b)).is_some() || controls.has(key) || (!controls.core_knows(port) && !controls.hidden(key));
                    let name = match controls.label(key) {
                        Some(l) if l != button_name(*b) => format!("{l} ({})", button_name(*b)),
                        _ => button_name(*b),
                    };
                    shown.then_some((name, Slot::Button(*b)))
                })
                .collect();
            // The directions of the sticks the controller has: a key pushes one all the way.
            for b in STICKS {
                let key = control_key(b, port);
                let Some((axis, positive)) = direction_of(key) else { continue };
                if self.get(Slot::Button(b)).is_none() && !controls.has(axis) && !controls.directions(axis) {
                    continue;
                }
                let way = match (axis.id, positive) {
                    (0, false) => t::STICK_LEFT,
                    (0, true) => t::STICK_RIGHT,
                    (_, false) => t::STICK_UP,
                    (_, true) => t::STICK_DOWN,
                };
                let name = match (controls.label(key), controls.label(axis)) {
                    // The project's own name for the direction (a C button), else the stick's and which way.
                    (Some(l), _) => format!("{l} ({})", button_name(b)),
                    (None, Some(a)) => format!("{a} {way} ({})", button_name(b)),
                    (None, None) => format!("{} {way}", button_name(b)),
                };
                slots.push((name, Slot::Button(b)));
            }
            slots.push((t::COMMIT_FRAME_BY_FRAME.into(), Slot::Commit));
            slots.push((t::REWIND_FRAME_BY_FRAME.into(), Slot::Rewind));
            egui::Grid::new("key_map").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
                for (label, slot) in slots {
                    ui.label(label);
                    let text = if self.waiting == Some(slot) { t::PRESS_A_KEY.to_string() } else { self.get(slot).map(|k| k.name().to_string()).unwrap_or_else(|| "—".into()) };
                    if ui.add(egui::Button::new(text).min_size(egui::vec2(110.0, 0.0)).selected(self.waiting == Some(slot))).clicked() {
                        self.waiting = Some(slot);
                    }
                    if ui.add_enabled(self.get(slot).is_some(), egui::Button::new("✖").small()).on_hover_text(t::NO_KEY_TIP).clicked() {
                        self.assign(slot, None);
                    }
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(t::RESET_TO_DEFAULT).clicked() {
                    self.draft = Bindings::default();
                    self.waiting = None;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t::SAVE_TITLE).clicked() {
                        out = Some(MappingResult::Save(self.draft.clone()));
                    }
                    if ui.button(t::CANCEL).clicked() {
                        out = Some(MappingResult::Cancel);
                    }
                });
            });
        });
        if !open {
            out = Some(MappingResult::Cancel);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u32) -> InputKey {
        InputKey { port: 0, device: DEVICE_JOYPAD, index: 0, id }
    }

    fn segment(spans: &[(InputKey, Vec<(u64, u64)>)]) -> Segment {
        let mut r = Segment::new("t", None, "t", 20);
        r.set_binary_spans_many(spans).unwrap();
        r
    }

    #[test]
    fn adds_a_press_and_merges() {
        let mut r = segment(&[(key(8), vec![(2, 5)])]);
        assert!(write_frame(&mut r, 5, &[key(8)], true, &none()).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(2, 6)]);
        assert!(write_frame(&mut r, 9, &[key(8)], true, &none()).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(2, 6), (9, 10)]);
        // Already held there: nothing to do.
        assert!(!write_frame(&mut r, 9, &[key(8)], true, &none()).unwrap());
    }

    #[test]
    fn overwrite_releases_everything_else() {
        let mut r = segment(&[(key(8), vec![(0, 10)]), (InputKey { port: 1, ..key(0) }, vec![(0, 20)])]);
        assert!(write_frame(&mut r, 4, &[key(4)], true, &none()).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(0, 4), (5, 10)]);
        assert_eq!(r.binary_spans(InputKey { port: 1, ..key(0) }), vec![(0, 4), (5, 20)]);
        assert_eq!(r.binary_spans(key(4)), vec![(4, 5)]);
    }

    #[test]
    fn without_overwrite_only_adds() {
        let mut r = segment(&[(key(8), vec![(0, 10)])]);
        let p2b = InputKey { port: 1, ..key(0) };
        assert!(write_frame(&mut r, 4, &[p2b], false, &none()).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(0, 10)]);
        assert_eq!(r.binary_spans(p2b), vec![(4, 5)]);
        assert!(!write_frame(&mut r, 6, &[], false, &none()).unwrap());
    }

    #[test]
    fn clear_at_the_edges() {
        let mut r = segment(&[(key(8), vec![(3, 4)])]);
        assert!(clear_frame(&mut r, 3, &none()).unwrap());
        assert!(r.binary_spans(key(8)).is_empty());
        assert!(!clear_frame(&mut r, 3, &none()).unwrap());
    }

    #[test]
    fn bindings_round_trip() {
        let b = Bindings::default();
        assert_eq!(Bindings::from_local(Some(&b.to_local())), b);
        let mut none = b.clone();
        none.commit = None;
        assert_eq!(Bindings::from_local(Some(&none.to_local())), none);
        let held: HashSet<Key> = [Key::W, Key::K, Key::Q].into_iter().collect();
        let mut c = b.chord(&held, 1);
        c.sort();
        assert_eq!(c, vec![InputKey { port: 1, ..key(4) }, InputKey { port: 1, ..key(8) }]);
    }

    fn none() -> BTreeSet<(u32, u32)> {
        BTreeSet::new()
    }

    #[test]
    fn stick_directions_have_names_and_keys() {
        // Every bindable control has a name that reads back as itself.
        for id in BUTTONS.into_iter().chain(STICKS) {
            assert_eq!(button_id(&button_name(id)), Some(id), "{}", button_name(id));
        }
        assert_eq!(button_name(16), "LEFT X-");
        assert_eq!(button_name(23), "RIGHT Y+");
        // [keyboard] round trip, and the chord on another controller.
        let mut b = Bindings::default();
        b.buttons.insert(button_id("RIGHT Y-").unwrap(), Key::ArrowUp);
        let back = Bindings::from_local(Some(&b.to_local()));
        assert_eq!(back, b);
        assert_eq!(b.stick_axes(), [(1, 1)].into_iter().collect());
        let chord = b.chord(&[Key::ArrowUp].into_iter().collect(), 2);
        assert_eq!(chord, vec![control_key(22, 2)]);
        assert_eq!(dj_core::format::input_text(chord[0], 1), "P3 ANALOG RIGHT Y -32767");
    }

    #[test]
    fn recording_a_stick_direction() {
        let y = InputKey { port: 0, device: DEVICE_ANALOG, index: 1, id: 1 };
        let x = InputKey { port: 0, device: DEVICE_ANALOG, index: 0, id: 0 };
        let up = direction(y, false);
        let sticks: BTreeSet<(u32, u32)> = [(1, 1)].into_iter().collect();
        let mut r = Segment::parse("dj-segment 1\nid: a\nparent: none\nframes: 20\n---\n0 P1 ANALOG LEFT X 5000\n").unwrap();
        // Held on frames 3 and 4: the stick goes all the way up, then back.
        assert!(write_frame(&mut r, 3, &[up], true, &sticks).unwrap());
        assert!(write_frame(&mut r, 4, &[up, key(8)], true, &sticks).unwrap());
        assert_eq!(r.values(y), vec![(3, -32767), (5, 0)]);
        // Recording over frame 4 with nothing held lets the stick go there, like a button,
        // and leaves the stick no key plays alone.
        assert!(write_frame(&mut r, 4, &[], true, &sticks).unwrap());
        assert_eq!(r.values(y), vec![(3, -32767), (4, 0)]);
        assert!(r.binary_spans(key(8)).is_empty());
        assert_eq!(r.values(x), vec![(0, 5000)]);
        // With no key on that stick, overwriting doesn't touch it.
        assert!(!write_frame(&mut r, 3, &[], true, &none()).unwrap());
        assert_eq!(r.values(y), vec![(3, -32767), (4, 0)]);
    }

}
