//! `scaffold::propose` and `InitPlan::write` (`dj init`), with the test core. Skipped when
//! there is no C compiler.

use dj_core::collection::Collection;
use dj_core::platform::ConsoleProfile;
use dj_core::scaffold::{propose, propose_with, FirstSegment, InitRequest};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh folder with a ROM in it, and the test core built with `flags`.
fn setup(what: &str, flags: &[&str]) -> Option<(PathBuf, PathBuf)> {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("scaffold-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    let lib = tmp.join("base.so");
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).arg("-DNO_DJ").args(flags).status().ok()?.success();
    if !ok {
        return None;
    }
    std::fs::write(tmp.join("My Game.bin"), b"rom").unwrap();
    Some((tmp, lib))
}

fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// One test: a process holds one core at a time.
#[test]
fn init() {
    a_project_from_a_rom_and_a_core();
    names_and_what_is_refused();
    a_core_without_a_connector_and_with_save_ram();
    console_defaults();
    the_core_for_the_roms_console();
    the_first_segment_and_gitignore();
}

fn a_project_from_a_rom_and_a_core() {
    let Some((tmp, lib)) = setup("plan", &["-DCONNECTOR"]) else { return };
    let rom = tmp.join("My Game.bin");
    let map = tmp.join("game.mlb");
    std::fs::write(&map, "").unwrap();
    let dir = tmp.join("proj");
    let req = InitRequest {
        dir: Some(dir.clone()),
        name: Some("A Game".into()),
        rom: rom.clone(),
        core: Some(lib.clone()),
        options: [("testcore_sync".to_string(), "fail".to_string())].into(),
        symbols: Some(map),
        ..Default::default()
    };
    let before = entries(&tmp);
    let plan = propose(&req).unwrap();

    // Proposing writes nothing: not the project's folder, nor anything beside it.
    assert!(!dir.exists());
    assert_eq!(entries(&tmp), before);

    let c = &plan.config;
    assert_eq!(c.name, "A Game");
    assert_eq!(c.rom.file, "My Game.bin");
    assert_eq!(c.rom.sha256, dj_core::collection::sha256_file(&rom).unwrap());
    assert_eq!(c.rom.shareable, None);
    assert_eq!(c.core.name, "testcore");
    assert!(!c.core.build.is_empty());
    // Every option the core reads is recorded, with the one asked for.
    assert_eq!(c.core.options.get("testcore_sync").map(String::as_str), Some("fail"));
    assert_eq!(c.hash.regions, ["RAM"]);
    // dj has no console defaults for the test core: a bare joypad, and no [cache].
    assert_eq!(plan.console, None);
    assert!(plan.notes.is_empty());
    assert_eq!(c.ports.len(), 1);
    assert!(c.ports[0].hide.is_empty() && c.ports[0].labels.is_empty());
    // The test core names no inputs, so there is nothing to count but the pad.
    assert_eq!(plan.controllers.text(), "1 joypad");
    assert!(c.cache.is_none());
    assert_eq!(c.symbols.len(), 1);
    assert_eq!((c.symbols[0].file.as_str(), c.symbols[0].format.as_str()), ("symbols/game.mlb", "mesen-mlb"));
    assert!(plan.connector);
    let prg = plan.regions.iter().find(|r| r.facts.name == "PRG").expect("the connector's PRG region");
    assert!(prg.decoder && prg.rolls_up && !prg.differs_from_rom);

    let made = plan.write().unwrap();
    assert_eq!(made.config, dir.join("config.toml"));
    assert!(made.segment.is_none());
    assert_eq!(made.ignored, [".local/"]);
    assert!(dir.join("segments").is_dir() && dir.join("blobs").is_dir() && dir.join("symbols/game.mlb").is_file());
    assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), ".local/\n");

    // It opens, says what the plan said, and knows where the ROM and the core are.
    let col = Collection::open(&dir).unwrap();
    assert_eq!(col.config.name, "A Game");
    assert_eq!(col.config.core.options, c.core.options);
    assert_eq!(col.config.core.build, c.core.build);
    let local = col.settings().unwrap();
    assert_eq!(local.rom, Some(std::fs::canonicalize(&rom).unwrap()));
    assert_eq!(local.core, Some(std::fs::canonicalize(&lib).unwrap()));

    // A folder that has a project is refused, by both steps.
    assert!(propose(&req).unwrap_err().to_string().contains("already has a project"));
    assert!(plan.write().unwrap_err().to_string().contains("already has a project"));

    // --core-build is recorded in place of what the core reports.
    let pinned = propose(&InitRequest { dir: Some(tmp.join("pinned")), core_build: Some("mine 2".into()), symbols: None, ..req.clone() }).unwrap();
    assert_eq!(pinned.config.core.build, "mine 2");
}

fn names_and_what_is_refused() {
    let Some((tmp, lib)) = setup("names", &["-DCONNECTOR"]) else { return };
    let rom = tmp.join("My Game.bin");
    let req = InitRequest { rom: rom.clone(), core: Some(lib.clone()), ..Default::default() };

    // No folder and no name: both from the ROM file's name.
    let plan = propose(&req).unwrap();
    assert_eq!(plan.dir, PathBuf::from("My Game"));
    assert_eq!(plan.config.name, "My Game");

    // A folder and no name: the folder's name, or the ROM's when the folder has none.
    let plan = propose(&InitRequest { dir: Some(tmp.join("port")), ..req.clone() }).unwrap();
    assert_eq!(plan.config.name, "port");
    let plan = propose(&InitRequest { dir: Some(tmp.join("nowhere/..")), ..req.clone() }).unwrap();
    assert_eq!(plan.config.name, "My Game");
    // `.` in a folder that's there is that folder.
    let plan = propose(&InitRequest { dir: Some(tmp.join(".")), ..req.clone() }).unwrap();
    assert_eq!(plan.config.name, tmp.file_name().unwrap().to_string_lossy());

    // The core's suffix is added when it's left off.
    let alt = tmp.join("alt").with_extension(std::env::consts::DLL_EXTENSION);
    std::fs::copy(&lib, &alt).unwrap();
    let plan = propose(&InitRequest { dir: Some(tmp.join("port")), core: Some(tmp.join("alt")), ..req.clone() }).unwrap();
    assert_eq!(plan.core, alt);

    let err = |r: InitRequest| propose(&r).unwrap_err().to_string();
    assert!(err(InitRequest { rom: tmp.join("none.bin"), ..req.clone() }).contains("the ROM"));
    assert!(err(InitRequest { core: Some(tmp.join("none_libretro")), ..req.clone() }).contains("the core library"));
    assert!(err(InitRequest { symbols: Some(tmp.join("none.mlb")), ..req.clone() }).contains("the symbol map"));
}

fn a_core_without_a_connector_and_with_save_ram() {
    let Some((tmp, lib)) = setup("plain", &["-DSAVE_RAM"]) else { return };
    let plan = propose(&InitRequest { dir: Some(tmp.join("proj")), rom: tmp.join("My Game.bin"), core: Some(lib), ..Default::default() }).unwrap();
    assert!(!plan.connector);
    assert!(plan.regions.is_empty());
    assert_eq!(plan.config.hash.regions, ["RAM", "SRAM"]);
    plan.write().unwrap();
    assert!(Collection::open(&tmp.join("proj")).is_ok());
}

/// A console for the test core: an option it has, one it doesn't, and bus ranges it
/// doesn't map (it publishes no memory maps).
static TOY: ConsoleProfile = ConsoleProfile {
    console: "TOY",
    hash_bus: &[(0x0, 0x3F)],
    hash_note: "hashed through the bus",
    hash_also: &[],
    cache_preset: Some("n64-sparse"),
    hide: &["X", "Y"],
    labels: &[("B", "Fire")],
    options: &[("testcore_sync", "fail"), ("testcore_missing", "1")],
    options_note: "why",
    advice: "mind the gap",
};

fn toy(core: &str, _rom: Option<&[u8]>) -> Option<&'static ConsoleProfile> {
    (core == "testcore").then_some(&TOY)
}

fn console_defaults() {
    let Some((tmp, lib)) = setup("console", &["-DCONNECTOR"]) else { return };
    let req = InitRequest { dir: Some(tmp.join("proj")), rom: tmp.join("My Game.bin"), core: Some(lib), ..Default::default() };

    let plan = propose_with(&req, toy).unwrap();
    let c = &plan.config;
    assert_eq!(plan.console, Some("TOY"));
    assert_eq!(c.ports[0].hide, ["X", "Y"]);
    assert_eq!(c.ports[0].labels.get("B").map(String::as_str), Some("Fire"));
    assert_eq!(c.cache.as_ref().and_then(|c| c.preset.as_deref()), Some("n64-sparse"));
    // The option the core has is set and recorded, and said; the one it doesn't have isn't.
    assert_eq!(c.core.options.get("testcore_sync").map(String::as_str), Some("fail"));
    assert!(!c.core.options.contains_key("testcore_missing"));
    assert_eq!(plan.notes, ["testcore_sync = \"fail\": why", "mind the gap"]);
    // Bus ranges the core doesn't map are left alone: RAM is hashed, and nothing is said.
    assert_eq!(c.hash.regions, ["RAM"]);

    // An option given with the request wins over the console's, with no note.
    let plan = propose_with(&InitRequest { options: [("testcore_sync".to_string(), "ok".to_string())].into(), ..req.clone() }, toy).unwrap();
    assert_eq!(plan.config.core.options.get("testcore_sync").map(String::as_str), Some("ok"));
    assert_eq!(plan.notes, ["mind the gap"]);

    // The project it writes opens, with the preset and the controls read back.
    plan.write().unwrap();
    let col = Collection::open(&tmp.join("proj")).unwrap();
    assert_eq!(col.config.ports[0].hide, ["X", "Y"]);
    assert_eq!(col.config.cache.as_ref().and_then(|c| c.preset.as_deref()), Some("n64-sparse"));
}

fn the_core_for_the_roms_console() {
    let Some((tmp, lib)) = setup("find", &["-DCONNECTOR"]) else { return };
    // A cores folder with the test core in it, under the name of the NES core.
    let cores = tmp.join("my cores");
    std::fs::create_dir_all(&cores).unwrap();
    let mesen = cores.join("mesen_libretro").with_extension(std::env::consts::DLL_EXTENSION);
    std::fs::copy(&lib, &mesen).unwrap();
    std::env::set_var(dj_core::platform::CORES_ENV, &cores);
    std::env::remove_var(dj_core::collection::CORE_ENV);
    let nes = tmp.join("game.bin");
    std::fs::write(&nes, b"NES\x1A\x02\x01 and so on").unwrap();
    let req = InitRequest { dir: Some(tmp.join("proj")), rom: nes.clone(), ..Default::default() };

    // No core asked for: the ROM's header says NES, and the NES core is in the folder.
    let plan = propose(&req).unwrap();
    assert_eq!(plan.core, mesen);
    assert_eq!(plan.core_chosen_for, Some("NES"));

    // A core asked for by name alone is looked for there too, and wasn't chosen by console.
    let plan = propose(&InitRequest { core: Some("mesen_libretro".into()), ..req.clone() }).unwrap();
    assert_eq!(plan.core, mesen);
    assert_eq!(plan.core_chosen_for, None);

    // $DJ_CORE is a core asked for.
    std::env::set_var(dj_core::collection::CORE_ENV, &lib);
    assert_eq!(propose(&req).unwrap().core, lib);
    std::env::remove_var(dj_core::collection::CORE_ENV);

    // A console whose core isn't there: the error names the core, the folders and the script.
    let n64 = tmp.join("game.z64");
    std::fs::write(&n64, [0x80, 0x37, 0x12, 0x40, 0, 0, 0, 0]).unwrap();
    let e = propose(&InitRequest { rom: n64, ..req.clone() }).unwrap_err().to_string();
    assert!(e.contains("an N64 ROM") && e.contains("mupen64plus_next_libretro") && e.contains("my cores") && e.contains("connectors/mupen64plus/build.sh"), "{e}");
    let e = propose(&InitRequest { core: Some("nothing_libretro".into()), ..req.clone() }).unwrap_err().to_string();
    assert!(e.contains("nothing_libretro") && e.contains("my cores"), "{e}");

    // A ROM that says nothing about its console.
    let e = propose(&InitRequest { rom: tmp.join("My Game.bin"), ..req.clone() }).unwrap_err().to_string();
    assert!(e.contains("can't tell which console") && e.contains("--core"), "{e}");
    std::env::remove_var(dj_core::platform::CORES_ENV);
}

fn the_first_segment_and_gitignore() {
    let Some((tmp, lib)) = setup("first", &["-DCONNECTOR"]) else { return };
    // The ROM is in the project's folder, which has a .gitignore with no newline at its end.
    let dir = tmp.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    let rom = dir.join("My [Game].bin");
    std::fs::write(&rom, b"rom").unwrap();
    std::fs::write(dir.join(".gitignore"), "build/").unwrap();
    let first = FirstSegment { title: "Power-on".into(), frames: 60 };
    let plan = propose(&InitRequest { dir: Some(dir.clone()), rom, core: Some(lib.clone()), segment: Some(first), ..Default::default() }).unwrap();
    // Proposing left the folder as it was.
    assert_eq!(entries(&dir), [".gitignore", "My [Game].bin"]);

    let made = plan.write().unwrap();
    // What was there is kept; .local/ and the ROM (matched as itself, not as a pattern) are added.
    assert_eq!(made.ignored, [".local/", "/My \\[Game].bin"]);
    assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "build/\n.local/\n/My \\[Game].bin\n");

    let (id, file) = made.segment.unwrap();
    assert!(file.is_file() && file.starts_with(dir.join("segments")));
    let col = Collection::open(&dir).unwrap();
    assert_eq!(col.segments.len(), 1);
    let seg = &col.segments[&id];
    assert_eq!((seg.title(), seg.frames().unwrap(), seg.parent()), ("Power-on", 60, None));

    // A ROM outside the folder isn't named, and a .gitignore that has the line gets nothing.
    let dir2 = tmp.join("other");
    std::fs::create_dir_all(&dir2).unwrap();
    std::fs::write(dir2.join(".gitignore"), ".local/\n").unwrap();
    let made = propose(&InitRequest { dir: Some(dir2.clone()), rom: tmp.join("My Game.bin"), core: Some(lib), ..Default::default() }).unwrap().write().unwrap();
    assert!(made.ignored.is_empty() && made.segment.is_none());
    assert_eq!(std::fs::read_to_string(dir2.join(".gitignore")).unwrap(), ".local/\n");
}
