//! `scaffold::propose` and `InitPlan::write` (`dj init`), with the test core. Skipped when
//! there is no C compiler.

use dj_core::collection::Collection;
use dj_core::scaffold::{propose, InitRequest};
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
        core: lib.clone(),
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
    assert_eq!(c.symbols.len(), 1);
    assert_eq!((c.symbols[0].file.as_str(), c.symbols[0].format.as_str()), ("symbols/game.mlb", "mesen-mlb"));
    assert!(plan.connector);
    let prg = plan.regions.iter().find(|r| r.facts.name == "PRG").expect("the connector's PRG region");
    assert!(prg.decoder && prg.rolls_up && !prg.differs_from_rom);

    let path = plan.write().unwrap();
    assert_eq!(path, dir.join("config.toml"));
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
    let req = InitRequest { rom: rom.clone(), core: lib.clone(), ..Default::default() };

    // No folder and no name: both from the ROM file's name.
    let plan = propose(&req).unwrap();
    assert_eq!(plan.dir, PathBuf::from("My Game"));
    assert_eq!(plan.config.name, "My Game");

    // A folder and no name: the folder's name, or the ROM's when the folder has none.
    let plan = propose(&InitRequest { dir: Some(tmp.join("port")), ..req.clone() }).unwrap();
    assert_eq!(plan.config.name, "port");
    let plan = propose(&InitRequest { dir: Some(tmp.join("..")), ..req.clone() }).unwrap();
    assert_eq!(plan.config.name, "My Game");

    // The core's suffix is added when it's left off.
    let alt = tmp.join("alt").with_extension(std::env::consts::DLL_EXTENSION);
    std::fs::copy(&lib, &alt).unwrap();
    let plan = propose(&InitRequest { dir: Some(tmp.join("port")), core: tmp.join("alt"), ..req.clone() }).unwrap();
    assert_eq!(plan.core, alt);

    let err = |r: InitRequest| propose(&r).unwrap_err().to_string();
    assert!(err(InitRequest { rom: tmp.join("none.bin"), ..req.clone() }).contains("the ROM"));
    assert!(err(InitRequest { core: tmp.join("none_libretro"), ..req.clone() }).contains("the core library"));
    assert!(err(InitRequest { symbols: Some(tmp.join("none.mlb")), ..req.clone() }).contains("the symbol map"));
}

fn a_core_without_a_connector_and_with_save_ram() {
    let Some((tmp, lib)) = setup("plain", &["-DSAVE_RAM"]) else { return };
    let plan = propose(&InitRequest { dir: Some(tmp.join("proj")), rom: tmp.join("My Game.bin"), core: lib, ..Default::default() }).unwrap();
    assert!(!plan.connector);
    assert!(plan.regions.is_empty());
    assert_eq!(plan.config.hash.regions, ["RAM", "SRAM"]);
    plan.write().unwrap();
    assert!(Collection::open(&tmp.join("proj")).is_ok());
}
