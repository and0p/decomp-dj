//! What a project's inputs are called: the names the core gives them, and the project's
//! own in `[[ports]]` (docs/config.md#ports), which win.
//!
//! Names are for people. Segment files keep libretro's (`P1 JOYPAD B 1`), so a label can
//! change without touching a segment or a hash.

use std::collections::{BTreeMap, BTreeSet};

use crate::project::PortConfig;
use crate::format::{direction, direction_of, InputKey, DEVICE_ANALOG, DEVICE_JOYPAD, JOYPAD};

/// A control's name in `[[ports]]`: a joypad button as segment lines spell it (`B`, `L2`),
/// a stick's axis (`LEFT X`, `LEFT Y`, `RIGHT X`, `RIGHT Y`), or one direction of an axis as
/// a button (`RIGHT X-` is the right stick pushed left, `RIGHT Y+` pushed down: minus is
/// left and up). None for other inputs.
pub fn control_name(key: InputKey) -> Option<String> {
    if let Some((axis, positive)) = direction_of(key) {
        return control_name(axis).map(|n| format!("{n}{}", if positive { '+' } else { '-' }));
    }
    match (key.device, key.index, key.id) {
        (DEVICE_JOYPAD, 0, id) => JOYPAD.get(id as usize).map(|b| b.to_string()),
        (DEVICE_ANALOG, stick @ (0 | 1), axis @ (0 | 1)) => Some(format!("{} {}", ["LEFT", "RIGHT"][stick as usize], ["X", "Y"][axis as usize])),
        _ => None,
    }
}

/// The input `name` means on `port` (from 0): the inverse of [`control_name`], ignoring case.
pub fn control_from_name(port: u32, name: &str) -> Option<InputKey> {
    let name = name.trim();
    if let Some(axis) = name.strip_suffix(['+', '-']) {
        return control_from_name(port, axis).filter(|k| is_axis(*k)).map(|k| direction(k, name.ends_with('+')));
    }
    let w: Vec<String> = name.split_whitespace().map(str::to_ascii_uppercase).collect();
    match w.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [b] => JOYPAD.iter().position(|j| *j == b).map(|id| InputKey { port, device: DEVICE_JOYPAD, index: 0, id: id as u32 }),
        [stick @ ("LEFT" | "RIGHT"), axis @ ("X" | "Y")] => Some(InputKey { port, device: DEVICE_ANALOG, index: (stick == "RIGHT") as u32, id: (axis == "Y") as u32 }),
        _ => None,
    }
}

/// A stick's axis: an analog input that isn't a button's pressure.
pub fn is_axis(key: InputKey) -> bool {
    key.device == DEVICE_ANALOG && key.index <= 1 && key.id <= 1
}

/// The X and Y axes of stick `index` (0 left, 1 right) of `port`.
pub fn stick_keys(port: u32, index: u32) -> (InputKey, InputKey) {
    (InputKey { port, device: DEVICE_ANALOG, index, id: 0 }, InputKey { port, device: DEVICE_ANALOG, index, id: 1 })
}

/// The names in `[[ports]]` that aren't controls, as warnings.
pub fn check(ports: &[PortConfig]) -> Vec<String> {
    let mut out = vec![];
    for p in ports {
        for name in p.labels.keys().chain(&p.hide) {
            if control_from_name(0, name).is_none() {
                out.push(format!("config.toml [[ports]] port {}: \"{name}\" isn't a control (a joypad button such as B or L2, a stick's axis such as LEFT X, or one of its directions such as RIGHT Y-)", p.port));
            }
        }
    }
    out
}

/// The names of a project's inputs, and which of them it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Controls {
    /// The core's names.
    core: BTreeMap<InputKey, String>,
    /// The project's, which win.
    labels: BTreeMap<InputKey, String>,
    hidden: BTreeSet<InputKey>,
    /// The console's units for the main stick, where dj can convert to them.
    units: Option<crate::tas::profile::StickUnits>,
}

impl Controls {
    /// From what the core says (`Core::input_names`, empty when it says nothing or no core
    /// is loaded) and the project's `[[ports]]`.
    pub fn new(core: &[(InputKey, String)], ports: &[PortConfig]) -> Controls {
        let mut c = Controls { core: core.iter().filter(|(_, n)| !n.trim().is_empty()).map(|(k, n)| (*k, n.trim().to_string())).collect(), ..Default::default() };
        for p in ports.iter().filter(|p| p.port > 0) {
            for (name, label) in &p.labels {
                if let Some(k) = control_from_name(p.port - 1, name) {
                    c.labels.insert(k, label.clone());
                }
            }
            c.hidden.extend(p.hide.iter().filter_map(|n| control_from_name(p.port - 1, n)));
        }
        c
    }

    /// With the console's units for the main stick (None: libretro's values).
    pub fn with_units(mut self, units: Option<crate::tas::profile::StickUnits>) -> Controls {
        self.units = units;
        self
    }

    /// The console's units for `axis`, if it is the main (left) stick and dj can convert.
    pub fn units(&self, axis: InputKey) -> Option<&crate::tas::profile::StickUnits> {
        self.units.as_ref().filter(|_| is_axis(axis) && axis.index == 0)
    }

    /// The furthest stick `index` of `port` goes on one axis, as people see it (`stick_shown`).
    pub fn stick_max(&self, port: u32, index: u32) -> i32 {
        self.units(stick_keys(port, index).0).map_or(crate::format::STICK_FULL as i32, |u| u.max())
    }

    /// A stick position as people see it, for libretro's `(x, y)`: the console's units where
    /// dj can convert, else libretro's; right and up are positive either way.
    pub fn stick_shown(&self, port: u32, index: u32, x: i16, y: i16) -> (i32, i32) {
        match self.units(stick_keys(port, index).0) {
            Some(u) => u.to_console(x, y),
            None => (x as i32, -(y as i32)),
        }
    }

    /// libretro's `(x, y)` for a stick position as people see it: the inverse of `stick_shown`.
    pub fn stick_raw(&self, port: u32, index: u32, x: i32, y: i32) -> (i16, i16) {
        let full = crate::format::STICK_FULL as i32;
        match self.units(stick_keys(port, index).0) {
            Some(u) => u.from_console(x, y),
            None => (x.clamp(-full, full) as i16, (-y).clamp(-full, full) as i16),
        }
    }

    /// What to call `key`: the project's label, else the core's name, else None (use
    /// libretro's).
    pub fn label(&self, key: InputKey) -> Option<&str> {
        self.labels.get(&key).or_else(|| self.core.get(&key)).map(String::as_str)
    }

    pub fn hidden(&self, key: InputKey) -> bool {
        self.hidden.contains(&key)
    }

    /// Whether the core named `port`'s controls. When it didn't, show the usual ones.
    pub fn core_knows(&self, port: u32) -> bool {
        self.core.keys().any(|k| k.port == port)
    }

    /// Whether the core or the project names `key` (and the project doesn't hide it).
    pub fn has(&self, key: InputKey) -> bool {
        !self.hidden(key) && (self.labels.contains_key(&key) || self.core.contains_key(&key))
    }

    /// Whether the project names a direction of `axis` as a button: the axis then shows as
    /// two buttons, not as a value.
    pub fn directions(&self, axis: InputKey) -> bool {
        [false, true].iter().any(|p| self.labels.contains_key(&direction(axis, *p)))
    }

    /// The stick axes `port` has, in order: left X and Y, then right.
    pub fn axes(&self, port: u32) -> Vec<InputKey> {
        let all = (0..2).flat_map(|index| (0..2).map(move |id| InputKey { port, device: DEVICE_ANALOG, index, id }));
        all.filter(|k| self.has(*k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(labels: &[(&str, &str)], hide: &[&str]) -> PortConfig {
        PortConfig { port: 1, device: "JOYPAD".into(), labels: labels.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), hide: hide.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn names_round_trip() {
        for name in ["B", "L2", "LEFT X", "RIGHT Y"] {
            let k = control_from_name(2, name).unwrap();
            assert_eq!(k.port, 2);
            assert_eq!(control_name(k).as_deref(), Some(name));
        }
        assert_eq!(control_from_name(0, "left  y"), control_from_name(0, "LEFT Y"));
        assert!(control_from_name(0, "C Up").is_none());
        let up = control_from_name(1, "right y-").unwrap();
        assert_eq!(direction_of(up), Some((control_from_name(1, "RIGHT Y").unwrap(), false)));
        assert_eq!(control_name(up).as_deref(), Some("RIGHT Y-"));
        assert!(control_from_name(0, "B+").is_none());
        let c = Controls::new(&[], &[port(&[("RIGHT Y-", "C Up")], &[])]);
        assert!(c.directions(control_from_name(0, "RIGHT Y").unwrap()) && !c.directions(control_from_name(0, "RIGHT X").unwrap()));
        assert_eq!(c.label(control_from_name(0, "RIGHT Y-").unwrap()), Some("C Up"));
        assert!(is_axis(control_from_name(0, "RIGHT X").unwrap()) && !is_axis(control_from_name(0, "A").unwrap()));
    }

    #[test]
    fn the_project_wins_over_the_core() {
        let b = control_from_name(0, "B").unwrap();
        let y = control_from_name(0, "Y").unwrap();
        let x = control_from_name(0, "X").unwrap();
        let lx = control_from_name(0, "LEFT X").unwrap();
        let core = vec![(b, "A Button".to_string()), (y, "B Button".to_string()), (x, "Unused".to_string()), (lx, "Control Stick X".to_string())];
        let c = Controls::new(&core, &[port(&[("B", "A")], &["X"])]);
        assert_eq!(c.label(b), Some("A"));
        assert_eq!(c.label(y), Some("B Button"));
        assert!(c.hidden(x) && !c.has(x) && c.has(y));
        assert_eq!(c.axes(0), vec![lx]);
        assert!(c.core_knows(0) && !c.core_knows(1));
        // Nothing known: no labels, nothing hidden.
        let none = Controls::new(&[], &[]);
        assert_eq!(none.label(b), None);
        assert!(!none.core_knows(0) && none.axes(0).is_empty());
    }

    #[test]
    fn stick_positions_as_people_see_them() {
        // No conversion: libretro's values, with up positive.
        let c = Controls::default();
        assert_eq!(c.stick_shown(0, 0, 100, -200), (100, 200));
        assert_eq!(c.stick_raw(0, 0, 100, 200), (100, -200));
        assert_eq!(c.stick_max(0, 0), 32767);
        // The N64's control stick: the console's units, there and back; the right stick has none.
        let c = Controls::default().with_units(crate::tas::profile::StickUnits::for_core("mupen64plus-next", &Default::default()));
        assert_eq!(c.stick_max(0, 0), 80);
        assert_eq!(c.stick_max(0, 1), 32767);
        for (x, y) in [(0, 0), (80, 0), (-35, 62), (10, -10)] {
            let (rx, ry) = c.stick_raw(0, 0, x, y);
            assert_eq!(c.stick_shown(0, 0, rx, ry), (x, y));
        }
    }

    #[test]
    fn bad_names_are_reported() {
        let w = check(&[port(&[("C Up", "C Up"), ("B", "A")], &["TURBO"])]);
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("\"C Up\"") && w[1].contains("\"TURBO\""));
    }
}
