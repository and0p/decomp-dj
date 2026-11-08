//! Recording inputs from the keyboard into the piano roll (docs/gui.md#recording):
//! key bindings, the Record dropdown's options, and writing a frame's input.

use anyhow::Result;
use decomp_dj::collection::{KeyboardLocal, RecordLocal};
use decomp_dj::format::{is_binary, Command, InputKey, Replay, DEVICE_JOYPAD, JOYPAD};
use egui::Key;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Joypad buttons in the order the mapping dialog lists them (libretro ids).
pub const BUTTONS: [u32; 12] = [4, 5, 6, 7, 8, 0, 9, 1, 2, 3, 10, 11];

/// Which key plays each button of the controller being recorded, plus the two
/// frame-by-frame keys.
#[derive(Clone, PartialEq, Debug)]
pub struct Bindings {
    /// Joypad button id to key.
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

fn button_id(name: &str) -> Option<u32> {
    JOYPAD.iter().position(|b| b.eq_ignore_ascii_case(name)).map(|i| i as u32)
}

pub fn button_name(id: u32) -> &'static str {
    JOYPAD.get(id as usize).copied().unwrap_or("?")
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
            buttons: self.buttons.iter().map(|(b, k)| (button_name(*b).to_string(), k.name().to_string())).collect(),
            commit: Some(self.commit.map(|k| k.name().to_string()).unwrap_or_default()),
            rewind: Some(self.rewind.map(|k| k.name().to_string()).unwrap_or_default()),
        }
    }

    /// The buttons of controller `port` whose keys are held.
    pub fn chord(&self, held: &HashSet<Key>, port: u32) -> Vec<InputKey> {
        self.buttons.iter().filter(|(_, k)| held.contains(k)).map(|(b, _)| InputKey { port, device: DEVICE_JOYPAD, index: 0, id: *b }).collect()
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

/// Every on/off input any line of `r` sets.
fn binary_keys(r: &Replay) -> BTreeSet<InputKey> {
    r.lines
        .iter()
        .filter_map(|l| match l.command {
            Command::Input { key, .. } if is_binary(key) => Some(key),
            _ => None,
        })
        .collect()
}

/// Writes frame `f` of `r`: the buttons in `chord` are held there. With `overwrite`, every
/// other on/off input (on every controller) is released on that frame; without it, nothing
/// is released. Returns whether anything changed.
pub fn write_frame(r: &mut Replay, f: u64, chord: &[InputKey], overwrite: bool) -> Result<bool> {
    let mut keys: BTreeSet<InputKey> = chord.iter().copied().collect();
    if overwrite {
        keys.extend(binary_keys(r));
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

/// Releases every on/off input on frame `f`.
pub fn clear_frame(r: &mut Replay, f: u64) -> Result<bool> {
    write_frame(r, f, &[], true)
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

    pub fn show(&mut self, ctx: &egui::Context) -> Option<MappingResult> {
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
        egui::Window::new("Keyboard mapping").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(egui::RichText::new("Keys for the controller being recorded. Click a key, then press the new one (Escape cancels).").weak());
            ui.add_space(4.0);
            let mut slots: Vec<(String, Slot)> = BUTTONS.iter().map(|b| (button_name(*b).to_string(), Slot::Button(*b))).collect();
            slots.push(("Commit (frame by frame)".into(), Slot::Commit));
            slots.push(("Rewind (frame by frame)".into(), Slot::Rewind));
            egui::Grid::new("key_map").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
                for (label, slot) in slots {
                    ui.label(label);
                    let text = if self.waiting == Some(slot) { "press a key…".to_string() } else { self.get(slot).map(|k| k.name().to_string()).unwrap_or_else(|| "—".into()) };
                    if ui.add(egui::Button::new(text).min_size(egui::vec2(110.0, 0.0)).selected(self.waiting == Some(slot))).clicked() {
                        self.waiting = Some(slot);
                    }
                    if ui.add_enabled(self.get(slot).is_some(), egui::Button::new("✕").small()).on_hover_text("No key").clicked() {
                        self.assign(slot, None);
                    }
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Reset to default").clicked() {
                    self.draft = Bindings::default();
                    self.waiting = None;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Save").clicked() {
                        out = Some(MappingResult::Save(self.draft.clone()));
                    }
                    if ui.button("Cancel").clicked() {
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

    fn replay(spans: &[(InputKey, Vec<(u64, u64)>)]) -> Replay {
        let mut r = Replay::new("t", None, "t", 20);
        r.set_binary_spans_many(spans).unwrap();
        r
    }

    #[test]
    fn adds_a_press_and_merges() {
        let mut r = replay(&[(key(8), vec![(2, 5)])]);
        assert!(write_frame(&mut r, 5, &[key(8)], true).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(2, 6)]);
        assert!(write_frame(&mut r, 9, &[key(8)], true).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(2, 6), (9, 10)]);
        // Already held there: nothing to do.
        assert!(!write_frame(&mut r, 9, &[key(8)], true).unwrap());
    }

    #[test]
    fn overwrite_releases_everything_else() {
        let mut r = replay(&[(key(8), vec![(0, 10)]), (InputKey { port: 1, ..key(0) }, vec![(0, 20)])]);
        assert!(write_frame(&mut r, 4, &[key(4)], true).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(0, 4), (5, 10)]);
        assert_eq!(r.binary_spans(InputKey { port: 1, ..key(0) }), vec![(0, 4), (5, 20)]);
        assert_eq!(r.binary_spans(key(4)), vec![(4, 5)]);
    }

    #[test]
    fn without_overwrite_only_adds() {
        let mut r = replay(&[(key(8), vec![(0, 10)])]);
        let p2b = InputKey { port: 1, ..key(0) };
        assert!(write_frame(&mut r, 4, &[p2b], false).unwrap());
        assert_eq!(r.binary_spans(key(8)), vec![(0, 10)]);
        assert_eq!(r.binary_spans(p2b), vec![(4, 5)]);
        assert!(!write_frame(&mut r, 6, &[], false).unwrap());
    }

    #[test]
    fn clear_at_the_edges() {
        let mut r = replay(&[(key(8), vec![(3, 4)])]);
        assert!(clear_frame(&mut r, 3).unwrap());
        assert!(r.binary_spans(key(8)).is_empty());
        assert!(!clear_frame(&mut r, 3).unwrap());
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
}
