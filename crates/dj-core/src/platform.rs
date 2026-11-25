//! What dj knows about a console and the core that plays it, for making a project
//! (`scaffold`): what to hash, how to store snapshots, what the controller is called and
//! which core options a project should start with. One entry per core with a connector.
//! The console itself comes from `tas::profile::platform_of`.

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

/// The profile for a core (a project's `[core] name`) playing this ROM, if dj has one.
/// The ROM tells a Game Boy game from a GBA one on mGBA.
pub fn for_core(core: &str, rom: Option<&[u8]>) -> Option<&'static ConsoleProfile> {
    let console = crate::tas::profile::platform_of(core, rom)?;
    let p = match core {
        "mesen" => &MESEN_NES,
        "mgba" => &MGBA_GBA,
        c if c.starts_with("mupen64plus") => &MUPEN_N64,
        _ => return None,
    };
    (p.console == console).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

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
