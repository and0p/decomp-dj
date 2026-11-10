//! How a console's buttons, as TAS movies name them, map to libretro inputs on one core
//! (.agents/TAS_PLAN.md, "Profiles").
//!
//! Movie columns are named the way BizHawk names them ("P1 A", "P1 X Axis", or "A" for a
//! one-player handheld). Most consoles map one column to one libretro input. The N64 on
//! Mupen64Plus-Next doesn't: its C buttons may be a stick, and its control stick goes
//! through the core's deadzone and sensitivity, so that profile works out what the core
//! will read.

use anyhow::{bail, Result};
use std::collections::{BTreeMap, HashMap};

use crate::format::{InputKey, DEVICE_ANALOG, DEVICE_JOYPAD};

/// libretro joypad ids (`RETRO_DEVICE_ID_JOYPAD_*`).
const B: u32 = 0;
const Y: u32 = 1;
const SELECT: u32 = 2;
const START: u32 = 3;
const UP: u32 = 4;
const DOWN: u32 = 5;
const LEFT: u32 = 6;
const RIGHT: u32 = 7;
const A: u32 = 8;
const X: u32 = 9;
const L: u32 = 10;
const R: u32 = 11;
const L2: u32 = 12;
const R2: u32 = 13;
/// `RETRO_DEVICE_INDEX_ANALOG_LEFT` / `_RIGHT`, `RETRO_DEVICE_ID_ANALOG_X` / `_Y`.
const STICK_L: u32 = 0;
const STICK_R: u32 = 1;
const AXIS_X: u32 = 0;
const AXIS_Y: u32 = 1;

const fn pad(port: u32, id: u32) -> InputKey {
    InputKey { port, device: DEVICE_JOYPAD, index: 0, id }
}

const fn stick(port: u32, index: u32, id: u32) -> InputKey {
    InputKey { port, device: DEVICE_ANALOG, index, id }
}

#[derive(Debug, Clone)]
enum Map {
    /// Column `i` is libretro input `keys[i]`, with the same value.
    Direct(Vec<InputKey>),
    /// Four N64 pads on Mupen64Plus-Next (columns: `N64_BUTTONS` then the two axes, per player).
    N64(N64Opts),
}

#[derive(Debug, Clone, Copy)]
struct N64Opts {
    /// `mupen64plus-alt-map`: the C buttons on face buttons instead of the right stick.
    alt: bool,
    /// `mupen64plus-astick-deadzone`, in libretro stick units.
    deadzone: i32,
    /// `mupen64plus-astick-sensitivity`, in percent.
    sensitivity: i32,
}

/// A console on one core.
#[derive(Debug, Clone)]
pub struct Profile {
    /// The console, as movies name it: "NES", "GBA", "N64", "SNES", "GB".
    pub platform: String,
    /// Every button the movie model knows for this console, in a fixed order.
    pub columns: Vec<String>,
    map: Map,
    /// Core options this mapping assumes, with the value it needs.
    required_options: Vec<(&'static str, &'static str)>,
}

fn players(prefix_players: &[u32], buttons: &[(&str, u32)]) -> (Vec<String>, Vec<InputKey>) {
    let mut cols = vec![];
    let mut keys = vec![];
    for &p in prefix_players {
        for (name, id) in buttons {
            cols.push(if p == 0 { name.to_string() } else { format!("P{p} {name}") });
            keys.push(pad(p.saturating_sub(1), *id));
        }
    }
    (cols, keys)
}

const NES_BUTTONS: [(&str, u32); 8] = [("Up", UP), ("Down", DOWN), ("Left", LEFT), ("Right", RIGHT), ("Start", START), ("Select", SELECT), ("B", B), ("A", A)];
const SNES_BUTTONS: [(&str, u32); 12] =
    [("Up", UP), ("Down", DOWN), ("Left", LEFT), ("Right", RIGHT), ("Select", SELECT), ("Start", START), ("Y", Y), ("B", B), ("X", X), ("A", A), ("L", L), ("R", R)];
const GB_BUTTONS: [(&str, u32); 8] = [("Up", UP), ("Down", DOWN), ("Left", LEFT), ("Right", RIGHT), ("Start", START), ("Select", SELECT), ("B", B), ("A", A)];
const GBA_BUTTONS: [(&str, u32); 10] =
    [("Up", UP), ("Down", DOWN), ("Left", LEFT), ("Right", RIGHT), ("Start", START), ("Select", SELECT), ("B", B), ("A", A), ("L", L), ("R", R)];
/// An N64 pad's buttons, in BizHawk's order; the columns add "X Axis" and "Y Axis" after.
pub const N64_BUTTONS: [&str; 14] = ["DPad U", "DPad D", "DPad L", "DPad R", "Start", "Z", "B", "A", "C Up", "C Down", "C Left", "C Right", "L", "R"];
const N64_COLUMNS: usize = N64_BUTTONS.len() + 2;

fn direct(platform: &str, (columns, keys): (Vec<String>, Vec<InputKey>), required: Vec<(&'static str, &'static str)>) -> Profile {
    Profile { platform: platform.into(), columns, map: Map::Direct(keys), required_options: required }
}

/// Does the ROM look like a Game Boy (Color) cartridge? Its header has the Nintendo logo
/// at 0x104.
pub fn is_gb_rom(rom: &[u8]) -> bool {
    rom.len() > 0x150 && rom[0x104..0x108] == [0xCE, 0xED, 0x66, 0x66]
}

/// The console a project's core plays, if movies are supported for it. `rom` tells a
/// Game Boy game from a GBA one on mGBA.
pub fn platform_of(core: &str, rom: Option<&[u8]>) -> Option<&'static str> {
    Some(match core {
        "mesen" | "fceumm" | "nestopia" | "quicknes" => "NES",
        "mgba" if rom.is_some_and(is_gb_rom) => "GB",
        "mgba" | "vba-m" | "gpsp" => "GBA",
        "gambatte" | "sameboy" | "gearboy" => "GB",
        c if c.starts_with("snes9x") || c.starts_with("bsnes") || c == "mesen-s" => "SNES",
        c if c.starts_with("mupen64plus") => "N64",
        _ => return None,
    })
}

/// The profile for a project's core and a movie's console. GBC movies play on a GB profile.
pub fn for_core(core: &str, platform: &str, options: &BTreeMap<String, String>) -> Result<Profile> {
    let platform = if platform == "GBC" { "GB" } else { platform };
    let supported = |p: &str| match p {
        "NES" => ["mesen", "fceumm", "nestopia", "quicknes"].contains(&core),
        "GBA" => ["mgba", "vba-m", "gpsp"].contains(&core),
        "GB" => ["mgba", "gambatte", "sameboy", "gearboy"].contains(&core),
        "SNES" => core.starts_with("snes9x") || core.starts_with("bsnes") || core == "mesen-s",
        "N64" => core.starts_with("mupen64plus"),
        _ => false,
    };
    if !supported(platform) {
        match platform_of(core, None) {
            Some(p) => bail!("this movie is for {platform}, and the project's core ({core}) plays {p}"),
            None => bail!("TAS movies aren't supported for the core `{core}` yet"),
        }
    }
    Ok(match platform {
        // On Mesen, X and Y are turbo buttons (which no movie has), and an option can move
        // A and B to libretro B and Y.
        "NES" => direct("NES", players(&[1, 2, 3, 4], &NES_BUTTONS), if core == "mesen" { vec![("mesen_shift_buttons_clockwise", "disabled")] } else { vec![] }),
        "SNES" => direct("SNES", players(&[1, 2], &SNES_BUTTONS), vec![]),
        "GB" => direct("GB", players(&[0], &GB_BUTTONS), vec![]),
        "GBA" => direct("GBA", players(&[0], &GBA_BUTTONS), vec![]),
        "N64" => {
            let opt = |k: &str| options.get(&format!("mupen64plus-{k}")).or_else(|| options.get(&format!("{core}-{k}"))).map(String::as_str);
            let pct = |k: &str, default: i32| opt(k).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(default);
            // The core works these out as `(int)(atoi(v) * 0.01f * 0x8000)`.
            let deadzone = (pct("astick-deadzone", 15) as f32 * 0.01f32 * 32768f32) as i32;
            let opts = N64Opts { alt: opt("alt-map") == Some("True"), deadzone, sensitivity: pct("astick-sensitivity", 100) };
            let mut columns = vec![];
            for p in 1..=4 {
                for b in N64_BUTTONS {
                    columns.push(format!("P{p} {b}"));
                }
                columns.push(format!("P{p} X Axis"));
                columns.push(format!("P{p} Y Axis"));
            }
            Profile { platform: "N64".into(), columns, map: Map::N64(opts), required_options: vec![] }
        }
        _ => unreachable!(),
    })
}

/// What a frame's movie values become, and anything lost on the way.
pub struct ToLibretro {
    /// Every libretro input this profile drives, with its value this frame (0 included).
    pub inputs: Vec<(InputKey, i16)>,
    /// The core can't read the values exactly as the movie has them (an N64 stick position
    /// its deadzone or sensitivity can't reach, or C-Left and C-Right at once on the stick).
    pub lossy: bool,
}

impl Profile {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// Every libretro input this profile drives.
    pub fn keys(&self) -> Vec<InputKey> {
        match &self.map {
            Map::Direct(keys) => keys.clone(),
            Map::N64(_) => (0..4).flat_map(|p| [B, Y, SELECT, START, UP, DOWN, LEFT, RIGHT, A, X, L, R, L2, R2].map(|id| pad(p, id)).into_iter().chain([stick(p, STICK_L, AXIS_X), stick(p, STICK_L, AXIS_Y), stick(p, STICK_R, AXIS_X), stick(p, STICK_R, AXIS_Y)])).collect(),
        }
    }

    /// The libretro inputs that play one frame of the movie (`values` in column order).
    pub fn to_libretro(&self, values: &[i16]) -> ToLibretro {
        match &self.map {
            Map::Direct(keys) => ToLibretro { inputs: keys.iter().copied().zip(values.iter().copied()).collect(), lossy: false },
            Map::N64(o) => {
                let mut inputs = vec![];
                let mut lossy = false;
                for p in 0..4u32 {
                    let v = &values[p as usize * N64_COLUMNS..(p as usize + 1) * N64_COLUMNS];
                    let (l, s) = n64_to_libretro(p, v, o);
                    inputs.extend(l);
                    lossy |= s;
                }
                ToLibretro { inputs, lossy }
            }
        }
    }

    /// The movie's values for what the core reads, `get` giving each held libretro input.
    pub fn from_libretro(&self, get: &dyn Fn(InputKey) -> i16) -> Vec<i16> {
        match &self.map {
            Map::Direct(keys) => keys.iter().map(|k| get(*k)).collect(),
            Map::N64(o) => (0..4u32).flat_map(|p| n64_from_libretro(p, get, o)).collect(),
        }
    }

    /// Warnings for core options that differ from what this mapping needs.
    pub fn check_options(&self, options: &BTreeMap<String, String>) -> Vec<String> {
        self.required_options
            .iter()
            .filter_map(|(k, v)| match options.get(*k) {
                Some(have) if have != v => Some(format!("the core option {k} is \"{have}\"; movies map buttons as if it were \"{v}\"")),
                _ => None,
            })
            .collect()
    }
}

/// Where the N64 buttons are in a player's columns.
fn n64_col(name: &str) -> usize {
    N64_BUTTONS.iter().position(|b| *b == name).unwrap()
}

/// The libretro joypad id for each N64 button that is a plain button on this mapping.
fn n64_buttons(alt: bool) -> Vec<(&'static str, u32)> {
    let mut m = vec![("DPad U", UP), ("DPad D", DOWN), ("DPad L", LEFT), ("DPad R", RIGHT), ("Start", START), ("Z", L2), ("A", B), ("B", Y)];
    if alt {
        m.extend([("C Right", R), ("C Left", L), ("C Down", A), ("C Up", X), ("R", R2), ("L", SELECT)]);
    } else {
        m.extend([("R", R), ("L", L)]);
    }
    m
}

/// The right stick's value past the core's C-button threshold (0x4000).
const C_STICK: i16 = 0x7FFF;

fn n64_to_libretro(p: u32, v: &[i16], o: &N64Opts) -> (Vec<(InputKey, i16)>, bool) {
    let on = |name: &str| v[n64_col(name)] != 0;
    let mut out: HashMap<InputKey, i16> = HashMap::new();
    for id in [B, Y, SELECT, START, UP, DOWN, LEFT, RIGHT, A, X, L, R, L2, R2] {
        out.insert(pad(p, id), 0);
    }
    for (name, id) in n64_buttons(o.alt) {
        if on(name) {
            out.insert(pad(p, id), 1);
        }
    }
    let mut lossy = false;
    let (mut cx, mut cy) = (0i16, 0i16);
    if !o.alt {
        // The core reads C-Left for a right stick left of -0x4000, C-Up above it, and so on:
        // only one of each pair at a time.
        match (on("C Left"), on("C Right")) {
            (true, true) => lossy = true,
            (true, false) => cx = -C_STICK,
            (false, true) => cx = C_STICK,
            _ => {}
        }
        match (on("C Up"), on("C Down")) {
            (true, true) => lossy = true,
            (true, false) => cy = -C_STICK,
            (false, true) => cy = C_STICK,
            _ => {}
        }
    }
    out.insert(stick(p, STICK_R, AXIS_X), cx);
    out.insert(stick(p, STICK_R, AXIS_Y), cy);
    let x = v[N64_BUTTONS.len()] as i32;
    let y = v[N64_BUTTONS.len() + 1] as i32;
    let ((ax, ay), exact) = n64_stick_input(x, y, o);
    lossy |= !exact;
    out.insert(stick(p, STICK_L, AXIS_X), ax);
    out.insert(stick(p, STICK_L, AXIS_Y), ay);
    let mut v: Vec<_> = out.into_iter().collect();
    v.sort();
    (v, lossy)
}

fn n64_from_libretro(p: u32, get: &dyn Fn(InputKey) -> i16, o: &N64Opts) -> Vec<i16> {
    let mut v = vec![0i16; N64_COLUMNS];
    let held = |id: u32| get(pad(p, id)) != 0;
    // Without alt-map, R2 switches the face buttons to C buttons (set by other core
    // options): rare in a segment, and read here as the plain buttons.
    for (name, id) in n64_buttons(o.alt) {
        if held(id) {
            v[n64_col(name)] = 1;
        }
    }
    let cx = get(stick(p, STICK_R, AXIS_X)) as i32;
    let cy = get(stick(p, STICK_R, AXIS_Y)) as i32;
    if cx.abs() > 0x4000 {
        v[n64_col(if cx < 0 { "C Left" } else { "C Right" })] = 1;
    }
    if cy.abs() > 0x4000 {
        v[n64_col(if cy < 0 { "C Up" } else { "C Down" })] = 1;
    }
    let (x, y) = n64_stick(get(stick(p, STICK_L, AXIS_X)), get(stick(p, STICK_L, AXIS_Y)), o);
    v[N64_BUTTONS.len()] = x as i16;
    v[N64_BUTTONS.len() + 1] = y as i16;
    v
}

/// What Mupen64Plus-Next makes of a left stick position (`inputGetKeys_reuse`): the N64's
/// X and Y, up positive, as the 8-bit signed values the controller sends.
fn n64_stick(ax: i16, ay: i16, o: &N64Opts) -> (i32, i32) {
    let (x, y) = (ax as i32, ay as i32);
    let mut radius = ((x * x + y * y) as f64).sqrt();
    let angle = (y as f64).atan2(x as f64);
    if radius <= o.deadzone as f64 {
        return (0, 0);
    }
    // `(float)ASTICK_MAX / (ASTICK_MAX - astick_deadzone)` is a float division in C.
    let factor = (32768f32 / (32768 - o.deadzone) as f32) as f64;
    radius = (radius - o.deadzone as f64) * factor;
    radius *= 80.0 / 32768.0 * (o.sensitivity as f64 / 100.0);
    let round = |v: f64| (v + 0.5).floor() as i32;
    // X_AXIS and Y_AXIS are 8-bit signed bitfields.
    ((round(radius * angle.cos()) as i8) as i32, ((-round(radius * angle.sin())) as i8) as i32)
}

/// A left stick position the core turns into N64 `(x, y)` exactly, if there is one near
/// the obvious guess; otherwise the closest one found, and false.
fn n64_stick_input(x: i32, y: i32, o: &N64Opts) -> ((i16, i16), bool) {
    if x == 0 && y == 0 {
        return ((0, 0), true);
    }
    let len = ((x * x + y * y) as f64).sqrt();
    let scale = 80.0 * o.sensitivity as f64 / 100.0;
    let r = o.deadzone as f64 + len * (32768 - o.deadzone) as f64 / scale;
    let (gx, gy) = (r * x as f64 / len, -r * y as f64 / len);
    let mut best = ((0i16, 0i16), i32::MAX);
    for dx in -4..=4 {
        for dy in -4..=4 {
            let ax = (gx.round() as i32 + dx).clamp(-32768, 32767) as i16;
            let ay = (gy.round() as i32 + dy).clamp(-32768, 32767) as i16;
            let (rx, ry) = n64_stick(ax, ay, o);
            let err = (rx - x).abs() + (ry - y).abs();
            if err < best.1 || (err == best.1 && (dx.abs() + dy.abs()) == 0) {
                best = ((ax, ay), err);
            }
            if err == 0 && dx == 0 && dy == 0 {
                return ((ax, ay), true);
            }
        }
    }
    (best.0, best.1 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n64(opts: &[(&str, &str)]) -> Profile {
        let o = opts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        for_core("mupen64plus-next", "N64", &o).unwrap()
    }

    #[test]
    fn n64_stick_round_trips_inside_the_cores_range() {
        let p = n64(&[("mupen64plus-astick-deadzone", "0"), ("mupen64plus-astick-sensitivity", "100")]);
        let Map::N64(o) = p.map else { panic!() };
        let mut misses = 0;
        for x in -80..=80 {
            for y in -80..=80 {
                if x * x + y * y > 80 * 80 {
                    continue;
                }
                let ((ax, ay), exact) = n64_stick_input(x, y, &o);
                if !exact || n64_stick(ax, ay, &o) != (x, y) {
                    misses += 1;
                }
            }
        }
        assert_eq!(misses, 0);
    }

    #[test]
    fn n64_buttons_round_trip_both_mappings() {
        for alt in ["False", "True"] {
            let p = n64(&[("mupen64plus-alt-map", alt), ("mupen64plus-astick-deadzone", "15")]);
            let mut values = vec![0i16; p.columns.len()];
            for b in ["A", "Z", "C Up", "C Left", "R", "DPad L"] {
                values[p.column(&format!("P1 {b}")).unwrap()] = 1;
            }
            values[p.column("P2 X Axis").unwrap()] = 40;
            values[p.column("P2 Y Axis").unwrap()] = -25;
            let lr = p.to_libretro(&values);
            assert!(!lr.lossy, "alt {alt}");
            let map: HashMap<InputKey, i16> = lr.inputs.into_iter().collect();
            let back = p.from_libretro(&|k| map.get(&k).copied().unwrap_or(0));
            assert_eq!(back, values, "alt {alt}");
        }
    }

    #[test]
    fn platforms() {
        assert_eq!(platform_of("mesen", None), Some("NES"));
        assert_eq!(platform_of("mgba", None), Some("GBA"));
        assert_eq!(platform_of("mupen64plus-next", None), Some("N64"));
        assert!(for_core("mesen", "N64", &BTreeMap::new()).is_err());
        assert_eq!(for_core("gambatte", "GBC", &BTreeMap::new()).unwrap().columns[0], "Up");
    }
}
