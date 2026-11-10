//! TAS movies: exporting a chain and importing it back (no core needed: neither plays).

use decomp_dj::collection::Collection;
use decomp_dj::format::{input_text, Command};
use decomp_dj::tas::{self, ExportEnd};
use std::path::{Path, PathBuf};

/// A tiny iNES file: header, 16 KB PRG, 8 KB CHR.
fn rom() -> Vec<u8> {
    let mut r = b"NES\x1a\x01\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
    r.extend((0..0x6000u32).map(|i| (i * 7) as u8));
    r
}

const ROOT: &str = "dj-segment 1\nid: aaaaaaaa-0000-0000-0000-000000000000\nparent: none\ntitle: Boot\nframes: 10\n---\n\
    2 P1 JOYPAD START 1\n4 P1 JOYPAD START 0\n6 P1 JOYPAD RIGHT 1\n8 [EVENT] Title screen\n";
/// Holds Right across its parent's end only by setting it again at frame 0, then resets.
const CHILD: &str = "dj-segment 1\nid: bbbbbbbb-0000-0000-0000-000000000000\nparent: aaaaaaaa-0000-0000-0000-000000000000\ntitle: Walk\nframes: 8\n---\n\
    1 P1 JOYPAD A 1\n1 P2 JOYPAD B 1\n3 [RESET]\n3 P1 JOYPAD A 0\n5 P2 JOYPAD B 0\n";
const POKED: &str = "dj-segment 1\nid: cccccccc-0000-0000-0000-000000000000\nparent: aaaaaaaa-0000-0000-0000-000000000000\ntitle: Poke\nframes: 4\n---\n\
    2 [WRITE] RAM 0x10 01\n";

fn project(tmp: &Path) -> Collection {
    let _ = std::fs::remove_dir_all(tmp);
    std::fs::create_dir_all(tmp.join("segments")).unwrap();
    std::fs::write(tmp.join("game.nes"), rom()).unwrap();
    std::fs::write(
        tmp.join("config.toml"),
        "format = 1\nname = \"Game\"\n[rom]\nsha256 = \"\"\npath = \"game.nes\"\n[core]\nname = \"mesen\"\nbuild = \"1\"\n\
         [core.options]\nmesen_shift_buttons_clockwise = \"disabled\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n",
    )
    .unwrap();
    for text in [ROOT, CHILD, POKED] {
        let id = text.lines().nth(1).unwrap().trim_start_matches("id: ");
        std::fs::write(tmp.join("segments").join(format!("{id}.segment")), text).unwrap();
    }
    Collection::open(tmp).unwrap()
}

fn import(col: &Collection, file: &Path, rom: &[u8], strict: bool) -> anyhow::Result<tas::Imported> {
    tas::import(&tas::Ctx { col, rom: Some(rom) }, file, None, strict)
}

/// The input, reset and event lines of a segment, as text.
fn lines_of(seg: &decomp_dj::format::Replay) -> Vec<String> {
    seg.lines
        .iter()
        .filter_map(|l| match &l.command {
            Command::Input { key, value, .. } => Some(format!("{} {}", l.frame, input_text(*key, *value))),
            Command::Reset => Some(format!("{} [RESET]", l.frame)),
            Command::Event { text, .. } if !text.starts_with("Imported") => Some(format!("{} [EVENT] {text}", l.frame)),
            _ => None,
        })
        .collect()
}

/// The pressed inputs on each frame of the chain, as the core sees them.
fn pressed(movie: &tas::Movie) -> Vec<Vec<String>> {
    movie.frames.iter().map(|f| movie.columns.iter().zip(&f.values).filter(|(_, v)| **v != 0).map(|(c, _)| c.clone()).collect()).collect()
}

#[test]
fn export_then_import_keeps_what_the_core_sees() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tas-{}", std::process::id()));
    let col = project(&tmp);
    let rom = rom();
    let end = ExportEnd { segment: "bbbbbbbb-0000-0000-0000-000000000000", through: None };
    let (movie, warnings) = tas::from_chain(&col, end, Some(&rom), true).unwrap();
    // Both segments are unrecorded: the export says so but goes ahead.
    assert!(warnings.iter().any(|w| w.contains("dirty")), "{warnings:?}");
    assert_eq!(movie.frames.len(), 18);
    let p = pressed(&movie);
    assert_eq!(p[2], vec!["P1 Start"]);
    assert_eq!(p[9], vec!["P1 Right"]);
    // The child starts with every input released, so Right is not held into it.
    assert!(p[10].is_empty());
    assert_eq!(p[11], vec!["P1 A", "P2 B"]);
    assert!(movie.frames[13].reset);
    assert_eq!(p[13], vec!["P2 B"]);
    assert_eq!(movie.markers, vec![(8, "Title screen".to_string())]);

    let ex = tas::export(&tas::Ctx { col: &col, rom: Some(&rom) }, ExportEnd { segment: "bbbb", through: None }, false, Some("tester")).unwrap();
    assert_eq!(ex.frames, 18);
    let file = tmp.join("chain.bk2");
    std::fs::write(&file, &ex.bytes).unwrap();
    let imp = import(&col, &file, &rom, true).unwrap();
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);
    let seg = &imp.replay;
    assert_eq!(seg.parent(), None);
    assert_eq!(seg.frames().unwrap(), 18);
    assert!(seg.get("tas-source").unwrap().starts_with("bk2 decomp-dj"), "{:?}", seg.get("tas-source"));
    assert_eq!(seg.get("tas-author"), Some("tester"));
    let lines = lines_of(seg);
    let expected = [
            "2 P1 JOYPAD START 1",
            "4 P1 JOYPAD START 0",
            "6 P1 JOYPAD RIGHT 1",
            "8 [EVENT] Title screen",
            "10 P1 JOYPAD RIGHT 0",
            "11 P1 JOYPAD A 1",
            "11 P2 JOYPAD B 1",
            "13 [RESET]",
            "13 P1 JOYPAD A 0",
            "15 P2 JOYPAD B 0",
    ];
    assert_eq!(lines, expected);

    // Only .bk2 is read and written.
    let ctx = tas::Ctx { col: &col, rom: Some(&rom) };
    assert!(tas::import(&ctx, &tmp.join("chain.fm2"), None, false).err().unwrap().to_string().contains(".bk2"));
    assert!(tas::check_ext(Path::new("out.mmo")).is_err());

    // Up to a playhead: frame 1 of the child is the last one in.
    let (short, _) = tas::from_chain(&col, ExportEnd { segment: "bbbb", through: Some(1) }, Some(&rom), true).unwrap();
    assert_eq!(short.frames.len(), 12);

    // A poked chain exports without its pokes, with one summary warning; strict refuses it.
    let (poked, warnings) = tas::from_chain(&col, ExportEnd { segment: "cccc", through: None }, Some(&rom), false).unwrap();
    assert_eq!(poked.frames.len(), 14);
    assert!(warnings.iter().any(|w| w.ends_with("contains:\n  * 1 memory write")), "{warnings:?}");
    let err = tas::from_chain(&col, ExportEnd { segment: "cccc", through: None }, Some(&rom), true).unwrap_err();
    assert!(format!("{err:#}").contains("1 memory write"), "{err:#}");

    // A root that starts from a snapshot is part of the same warning.
    let mut col = col;
    col.replays.get_mut("aaaaaaaa-0000-0000-0000-000000000000").unwrap().set("snapshot", "0123456789abcdef0123456789abcdef");
    let (_, warnings) = tas::from_chain(&col, ExportEnd { segment: "cccc", through: None }, Some(&rom), false).unwrap();
    assert!(warnings.iter().any(|w| w.contains("because the chain starts from a snapshot and contains:\n  * 1 memory write")), "{warnings:?}");
    let (_, warnings) = tas::from_chain(&col, ExportEnd { segment: "aaaa", through: None }, Some(&rom), false).unwrap();
    assert!(warnings.iter().any(|w| w.ends_with("because the chain starts from a snapshot")), "{warnings:?}");
    // A movie for another ROM is refused when strict.
    let mut other = rom.clone();
    other[100] ^= 1;
    assert!(import(&col, &file, &other, true).is_err());
    let loose = import(&col, &file, &other, false).unwrap();
    assert!(loose.warnings.iter().any(|w| w.contains("different ROM")));
    let _ = std::fs::remove_dir_all(&tmp);
}
