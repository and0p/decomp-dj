//! What dj knows about a console and the core that plays it, for making a project
//! (`scaffold`): what to hash, how to store snapshots, what the controller is called and
//! which core options a project should start with. One entry per core with a connector.
//! The console itself comes from `tas::profile::platform_of`.

use std::path::{Path, PathBuf};

/// A core's defaults for a new project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsoleProfile {
    /// The console, as `tas::profile::platform_of` names it ("NES").
    pub console: &'static str,
    /// Bus ranges to hash in place of `RAM`, as `(start, end)` addresses, when the core's
    /// `RAM` isn't the console's whole memory. Used only if the core maps them.
    pub hash_bus: &'static [(u64, u64)],
    /// Why `hash_bus` is used, for the person making the project.
    pub hash_note: &'static str,
    /// The `[cache]` preset, when one suits the console's snapshot size.
    pub cache_preset: Option<&'static str>,
    /// Controls the core names that the console's controller doesn't have.
    pub hide: &'static [&'static str],
    /// What the controller's inputs are called, where the core's names aren't the console's.
    pub labels: &'static [(&'static str, &'static str)],
    /// Core options a project starts with, unless it is given another value: `(key, value)`.
    pub options: &'static [(&'static str, &'static str)],
    /// Why `options` are set, and what turning them back costs.
    pub options_note: &'static str,
    /// Something about the console that a project's author should know from the start.
    pub advice: &'static str,
}

/// A profile that sets nothing, to build others from.
pub const NONE: ConsoleProfile = ConsoleProfile { console: "", hash_bus: &[], hash_note: "", cache_preset: None, hide: &[], labels: &[], options: &[], options_note: "", advice: "" };

/// Mesen also names turbo buttons and Famicom Disk System and VS System controls.
const MESEN_NES: ConsoleProfile = ConsoleProfile { console: "NES", cache_preset: Some("nes-sparse"), hide: &["Y", "X", "L", "R", "L2", "R2", "L3"], ..NONE };

/// mGBA also names turbo buttons and the solar sensor.
const MGBA_GBA: ConsoleProfile = ConsoleProfile {
    console: "GBA",
    // IWRAM, then EWRAM.
    hash_bus: &[(0x0300_0000, 0x0300_7FFF), (0x0200_0000, 0x0203_FFFF)],
    hash_note: "IWRAM and EWRAM are hashed through the core's memory maps: mGBA's RAM region is only the first 32 KB of EWRAM",
    hide: &["Y", "X", "L2", "R2", "L3", "R3"],
    ..NONE
};

/// The C buttons are the right stick pushed all the way, unless "C Buttons Mode" (R2) is
/// held, which puts them on B, Y, A and X instead. Named as directions, they are button rows.
const MUPEN_N64: ConsoleProfile = ConsoleProfile {
    console: "N64",
    cache_preset: Some("n64-sparse"),
    hide: &["A", "X", "R2"],
    labels: &[
        ("B", "A"),
        ("Y", "B"),
        ("L2", "Z"),
        ("L", "L"),
        ("R", "R"),
        ("LEFT X", "Stick X"),
        ("LEFT Y", "Stick Y"),
        ("RIGHT Y-", "C Up"),
        ("RIGHT Y+", "C Down"),
        ("RIGHT X-", "C Left"),
        ("RIGHT X+", "C Right"),
    ],
    // The core's own options are kept, as the ones most games work with. That leaves the
    // GPU's copies to RDRAM on (EnableCopyColorToRDRAM, EnableCopyDepthToRDRAM).
    advice: "the core's options are kept as they are, the ones most games work with. With them the GPU copies its picture into the RAM dj hashes, and that copy isn't the same after a jump as from power-on: if a segment's hash differs after a jump, leave the game's framebuffers out with [hash] exclude and [cache] exclude (docs/config.md#console-defaults)",
    ..NONE
};

/// Mesen-S (SNES). Its RAM is the console's 128 KB of WRAM; Game Boy games, which it also
/// plays, aren't SNES projects. RAM is filled with zeros at power-on (the connector forces
/// it too), so runs are the same on every machine.
const MESENS_SNES: ConsoleProfile = ConsoleProfile {
    console: "SNES",
    hide: &["L2", "R2", "L3", "R3"],
    options: &[("mesen-s_ramstate", "All 0s")],
    options_note: "RAM starts as zeros at power-on instead of random bytes, so every run hashes the same",
    ..NONE
};

/// The profile for a core (a project's `[core] name`) playing this ROM, if dj has one.
/// The ROM tells a Game Boy game from a GBA one on mGBA.
pub fn for_core(core: &str, rom: Option<&[u8]>) -> Option<&'static ConsoleProfile> {
    let console = crate::tas::profile::platform_of(core, rom)?;
    let p = match core {
        "mesen" => &MESEN_NES,
        "mgba" => &MGBA_GBA,
        "mesen-s" => &MESENS_SNES,
        c if c.starts_with("mupen64plus") => &MUPEN_N64,
        _ => return None,
    };
    (p.console == console).then_some(p)
}

/// The console a ROM file is for, by its header, else by its file name's extension. Only
/// the consoles `core_for` has a core for. `head` is the file's first bytes (0x10200 is
/// enough for a SNES header).
pub fn detect(head: &[u8], path: &Path) -> Option<&'static str> {
    let at = |o: usize, bytes: &[u8]| head.get(o..o + bytes.len()) == Some(bytes);
    // iNES and NES 2.0.
    if at(0, b"NES\x1A") {
        return Some("NES");
    }
    // The N64 header's first word, in the three byte orders dumps come in (.z64, .v64, .n64).
    if at(0, &[0x80, 0x37, 0x12, 0x40]) || at(0, &[0x37, 0x80, 0x40, 0x12]) || at(0, &[0x40, 0x12, 0x37, 0x80]) {
        return Some("N64");
    }
    // The GBA header: the start of the Nintendo logo, and the fixed byte after the title.
    if at(4, &[0x24, 0xFF, 0xAE, 0x51]) && at(0xB2, &[0x96]) {
        return Some("GBA");
    }
    // The SNES header (at $7FC0 for LoROM, $FFC0 for HiROM, 512 bytes on with a copier
    // header): a map mode byte of $2x or $3x, and a checksum and its complement that add up
    // to $FFFF.
    let snes = |h: usize| head.get(h + 0x15).is_some_and(|m| (0x20..0x40).contains(m)) && head.get(h + 0x1C..h + 0x20).is_some_and(|c| u16::from_le_bytes([c[0], c[1]]) ^ u16::from_le_bytes([c[2], c[3]]) == 0xFFFF && c != [0xFF, 0xFF, 0, 0]);
    if [0x7FC0, 0xFFC0, 0x81C0, 0x101C0].into_iter().any(snes) {
        return Some("SNES");
    }
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "nes" => Some("NES"),
        "sfc" | "smc" => Some("SNES"),
        "z64" | "v64" | "n64" => Some("N64"),
        "gba" => Some("GBA"),
        _ => None,
    }
}

/// The core dj makes a console's projects with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreChoice {
    /// What people call the core ("Mesen").
    pub title: &'static str,
    /// The library's file name without its suffix (`mesen_libretro`).
    pub library: &'static str,
    /// The script that builds it with its connector, from the top of dj's folder.
    pub build: &'static str,
}

/// The core for a console `detect` names.
pub fn core_for(console: &str) -> Option<CoreChoice> {
    Some(match console {
        "NES" => CoreChoice { title: "Mesen", library: "mesen_libretro", build: "connectors/mesen/build.sh" },
        "GBA" => CoreChoice { title: "mGBA", library: "mgba_libretro", build: "connectors/mgba/build.sh" },
        "N64" => CoreChoice { title: "Mupen64Plus-Next", library: "mupen64plus_next_libretro", build: "connectors/mupen64plus/build.sh" },
        "SNES" => CoreChoice { title: "Mesen-S", library: "mesen-s_libretro", build: "connectors/mesen-s/build.sh" },
        _ => return None,
    })
}

/// A folder of cores, for a machine that keeps them somewhere of its own.
pub const CORES_ENV: &str = "DJ_CORES";

/// Where a core named without a folder is looked for, in order: `$DJ_CORES`; `cores/` next
/// to the running program (a release's folder); `build/cores/` two folders above it (a
/// source checkout, where the program is in `target/release/` and the connectors' scripts
/// build there); and `build/cores/` in the current folder.
pub fn core_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(d) = std::env::var_os(CORES_ENV).filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(d));
    }
    if let Some(exe) = std::env::current_exe().ok().and_then(|e| std::fs::canonicalize(e).ok()) {
        // A program inside a macOS app bundle (X.app/Contents/MacOS/) is where the bundle is.
        let in_bundle = exe.parent().filter(|d| d.ends_with("Contents/MacOS")).and_then(|d| d.parent()?.parent()).filter(|app| app.extension().is_some_and(|e| e == "app")).and_then(Path::parent);
        if let Some(dir) = in_bundle.or(exe.parent()) {
            dirs.push(dir.join("cores"));
            if let Some(top) = dir.parent().and_then(Path::parent) {
                dirs.push(top.join("build").join("cores"));
            }
        }
    }
    dirs.push(PathBuf::from("build").join("cores"));
    dirs
}

/// Every core in `dirs`: files named `*_libretro` with this system's library suffix, each
/// name once (the first folder that has it wins, as `find_core` finds it).
pub fn cores_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = vec![];
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        let mut here: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e == std::env::consts::DLL_EXTENSION) && p.file_stem().is_some_and(|s| s.to_string_lossy().ends_with("_libretro"))).collect();
        here.sort();
        for p in here {
            if seen.insert(p.file_name().map(|n| n.to_os_string())) {
                out.push(p);
            }
        }
    }
    out
}

/// What to call a core's file: the name of a core dj has a console for ("Mesen"), else
/// the file's name without `_libretro` and its suffix.
pub fn core_title(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for console in ["NES", "GBA", "N64", "SNES"] {
        if let Some(c) = core_for(console).filter(|c| c.library == stem) {
            return c.title.to_string();
        }
    }
    stem.strip_suffix("_libretro").unwrap_or(&stem).to_string()
}

/// The file of the core called `library` (with or without its suffix) in the first of
/// `dirs` that has it.
pub fn find_core(library: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| crate::libretro::library_path(&d.join(library))).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consoles_from_rom_files() {
        let p = |name: &str| PathBuf::from(name);
        let mut gba = vec![0u8; 0xC0];
        gba[4..8].copy_from_slice(&[0x24, 0xFF, 0xAE, 0x51]);
        gba[0xB2] = 0x96;
        // By header, whatever the file is called.
        assert_eq!(detect(b"NES\x1A\x02\x01", &p("game.bin")), Some("NES"));
        assert_eq!(detect(&[0x80, 0x37, 0x12, 0x40, 0, 0], &p("game.rom")), Some("N64"));
        assert_eq!(detect(&[0x37, 0x80, 0x40, 0x12, 0, 0], &p("game")), Some("N64"));
        assert_eq!(detect(&[0x40, 0x12, 0x37, 0x80, 0, 0], &p("game")), Some("N64"));
        assert_eq!(detect(&gba, &p("game.bin")), Some("GBA"));
        // By extension, when the header says nothing (a short or headerless file).
        assert_eq!(detect(b"rom", &p("game.NES")), Some("NES"));
        assert_eq!(detect(b"rom", &p("game.v64")), Some("N64"));
        assert_eq!(detect(b"rom", &p("game.gba")), Some("GBA"));
        assert_eq!(detect(b"rom", &p("game.bin")), None);
        // SNES: a LoROM header, and by extension.
        let mut sfc = vec![0u8; 0x8000];
        sfc[0x7FD5] = 0x20;
        sfc[0x7FDC..0x7FE0].copy_from_slice(&[0x34, 0x12, 0xCB, 0xED]);
        assert_eq!(detect(&sfc, &p("game.bin")), Some("SNES"));
        assert_eq!(detect(b"rom", &p("game.sfc")), Some("SNES"));
        assert_eq!(core_for("SNES").map(|c| c.library), Some("mesen-s_libretro"));
        assert_eq!(for_core("mesen-s", None).map(|p| p.console), Some("SNES"));
        assert_eq!(detect(b"", &p("game")), None);
        // Every console detected has a core, and that core's profile is for the console.
        for c in ["NES", "GBA", "N64", "SNES"] {
            assert!(core_for(c).is_some(), "{c}");
        }
        assert_eq!(core_for("Saturn"), None);
    }

    #[test]
    fn cores_in_folders() {
        let tmp = std::env::temp_dir().join(format!("dj-cores-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let ext = std::env::consts::DLL_EXTENSION;
        for (dir, file) in [("a", format!("mesen_libretro.{ext}")), ("a", "notes.txt".to_string()), ("a", format!("helper.{ext}")), ("b", format!("mesen_libretro.{ext}")), ("b", format!("fceumm_libretro.{ext}"))] {
            std::fs::create_dir_all(tmp.join(dir)).unwrap();
            std::fs::write(tmp.join(dir).join(file), "").unwrap();
        }
        let dirs = [tmp.join("a"), tmp.join("missing"), tmp.join("b")];
        // Only cores, each name once, the first folder's copy.
        assert_eq!(cores_in(&dirs), [tmp.join("a").join(format!("mesen_libretro.{ext}")), tmp.join("b").join(format!("fceumm_libretro.{ext}"))]);
        assert_eq!(core_title(&dirs[0].join(format!("mesen_libretro.{ext}"))), "Mesen");
        assert_eq!(core_title(&dirs[2].join(format!("fceumm_libretro.{ext}"))), "fceumm");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn profiles_by_core_and_rom() {
        assert_eq!(for_core("mesen", None).map(|p| p.console), Some("NES"));
        assert_eq!(for_core("mupen64plus-next", None).map(|p| p.console), Some("N64"));
        assert_eq!(for_core("mgba", None).map(|p| p.console), Some("GBA"));
        // A Game Boy game on mGBA isn't a GBA project: its RAM region is the right one.
        let mut gb = vec![0u8; 0x200];
        gb[0x104..0x108].copy_from_slice(&[0xCE, 0xED, 0x66, 0x66]);
        assert_eq!(for_core("mgba", Some(&gb)), None);
        // A core dj knows the console of, but has no defaults for; and one it doesn't know.
        assert_eq!(for_core("fceumm", None), None);
        assert_eq!(for_core("testcore", None), None);
    }

    #[test]
    fn presets_and_controls_exist() {
        for p in [&MESEN_NES, &MGBA_GBA, &MUPEN_N64] {
            if let Some(name) = p.cache_preset {
                assert!(crate::cache::PRESETS.contains(&name), "{name}");
            }
            for c in p.hide.iter().chain(p.labels.iter().map(|(k, _)| k)) {
                assert!(crate::controls::control_from_name(0, c).is_some(), "{c}");
            }
        }
    }
}
