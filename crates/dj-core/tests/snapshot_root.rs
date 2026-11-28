//! Root segments that start from a save state (`Session::add_root`, `Session::state_at`,
//! `edit::carried_lines`), end to end with the test core, whose RAM depends on the buttons
//! held on every frame. Skipped when there is no C compiler.

use dj_core::project::Project;
use dj_core::edit::carried_lines;
use dj_core::player::{NewRoot, PlayOptions, Session};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 40 frames: B held from frame 3 to 20 (across the cut at 10), Y from 25 to 30.
const BODY: &str = "3 P1 JOYPAD B 1\n20 P1 JOYPAD B 0\n25 P1 JOYPAD Y 1\n30 P1 JOYPAD Y 0\n";

fn project(what: &str) -> Option<(Project, PathBuf)> {
    project_with(what, &[])
}

fn project_with(what: &str, flags: &[&str]) -> Option<(Project, PathBuf)> {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("snapshot-root-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("segments")).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    let lib = tmp.join("base.so");
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).args(["-DCONNECTOR", "-DNO_DJ"]).args(flags).status().ok()?.success();
    if !ok {
        return None;
    }
    std::fs::write(tmp.join("rom.bin"), b"rom").unwrap();
    std::fs::write(tmp.join("segments/seg.segment"), format!("dj-segment 1\nid: seg\nparent: none\nframes: 40\n---\n{BODY}")).unwrap();
    let path = lib.display().to_string().replace('\\', "/");
    std::fs::write(tmp.join("config.toml"), format!("format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"testcore 1\"\npath = \"{path}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n")).unwrap();
    Some((Project::open(&tmp).unwrap(), lib))
}

fn blobs(col: &Project) -> usize {
    std::fs::read_dir(col.root.join("blobs")).map_or(0, |d| d.count())
}

/// One test: a process holds one core at a time.
#[test]
fn roots_from_a_snapshot() {
    a_root_cut_from_a_frame_ends_where_the_segment_does();
    a_file_the_core_cannot_load_leaves_nothing_behind();
    a_root_from_exported_save_ram();
}

fn a_root_cut_from_a_frame_ends_where_the_segment_does() {
    let Some((mut col, lib)) = project("cut") else { return };
    let mut s = Session::open_with(&col, &col.root.join("rom.bin"), &lib, false).unwrap();
    let whole = s.play_chain(&col, "seg", &PlayOptions::default(), false).unwrap().pop().unwrap();

    // The state at frame 10, with B held there.
    let (state, at) = s.state_at(&col, "seg", 10).unwrap();
    assert_eq!(at, 10);
    let lines = carried_lines(&col, "seg", 10).unwrap();
    assert_eq!(lines, ["P1 JOYPAD B 1"]);
    let id = s.add_root(&mut col, &NewRoot { title: "cut".into(), frames: 30, snapshot: Some(state.clone()), start_frame: at, lines, ..Default::default() }).unwrap();

    // Saved, with the blob in blobs/ and the state it starts in recorded.
    let saved = Project::open(&col.root).unwrap();
    let r = &saved.segments[&id];
    assert_eq!((r.parent(), r.get("start-frame")), (None, Some("10")));
    assert_eq!(saved.read_blob(r.get("snapshot").unwrap(), "snapshot").unwrap(), state);
    // Named after the title, with the start of the content's hash.
    let hash = dj_core::project::blob_id(&state);
    assert_eq!(r.get("snapshot"), Some(hash.as_str()), "the header names the content");
    assert!(saved.blob_path(&hash, "snapshot").unwrap().ends_with(format!("blobs/cut-{}.state", &hash[..8])));
    assert!(r.get("start-hash").is_some());

    // The rest of the segment's lines, moved back by the cut, reach the same ending: the
    // held button came along. Without it the ending differs.
    let r = col.segments.get_mut(&id).unwrap();
    for (f, l) in [(10, "P1 JOYPAD B 0"), (15, "P1 JOYPAD Y 1"), (20, "P1 JOYPAD Y 0")] {
        r.insert_line(f, l).unwrap();
    }
    let cut = s.play_chain(&col, &id, &PlayOptions::default(), false).unwrap().pop().unwrap();
    assert_eq!(cut.end_hash, whole.end_hash);
    assert_eq!(Some(cut.start_hash.as_str()), col.segments[&id].get("start-hash"));
    assert_eq!(cut.start_frame, 10);

    let bare = s.add_root(&mut col, &NewRoot { frames: 30, snapshot: Some(state), start_frame: at, blob_name: "cut".into(), ..Default::default() }).unwrap();
    let r = col.segments.get_mut(&bare).unwrap();
    for (f, l) in [(10, "P1 JOYPAD B 0"), (15, "P1 JOYPAD Y 1"), (20, "P1 JOYPAD Y 0")] {
        r.insert_line(f, l).unwrap();
    }
    let other = s.play_chain(&col, &bare, &PlayOptions::default(), false).unwrap().pop().unwrap();
    assert_ne!(other.end_hash, whole.end_hash);
    assert_eq!(blobs(&col), 1, "the same state is one blob");

    // With the database: recorded once, then from an empty database, the snapshot root
    // reproduces its hashes, and a play from a stored state of it agrees.
    col.save_segment(&id).unwrap();
    drop(s);
    for round in 0..2 {
        let mut s = Session::open_with(&col, &col.root.join("rom.bin"), &lib, true).unwrap();
        let again = s.play_chain(&col, &id, &PlayOptions { record: true, ..Default::default() }, false).unwrap().pop().unwrap();
        assert_eq!((again.start_hash.as_str(), again.end_hash.as_str()), (cut.start_hash.as_str(), cut.end_hash.as_str()), "round {round}");
        let (late, at) = s.state_at(&col, &id, 25).unwrap();
        assert_eq!(at, 35);
        s.load_state(&late).unwrap();
        drop(s);
        if round == 0 {
            std::fs::remove_dir_all(col.root.join(".local")).unwrap();
        }
    }
}

fn a_file_the_core_cannot_load_leaves_nothing_behind() {
    let Some((mut col, lib)) = project("bad") else { return };
    let mut s = Session::open_with(&col, &col.root.join("rom.bin"), &lib, false).unwrap();
    let before = col.segments.len();
    let err = s.add_root(&mut col, &NewRoot { frames: 30, snapshot: Some(b"not a state".to_vec()), ..Default::default() }).unwrap_err();
    assert!(format!("{err:#}").contains("testcore 1"), "{err:#}");
    assert_eq!((col.segments.len(), blobs(&col)), (before, 0));
    assert_eq!(std::fs::read_dir(col.root.join("segments")).unwrap().count(), 1);
    assert!(s.add_root(&mut col, &NewRoot { frames: 30, ..Default::default() }).is_err(), "nothing to start from");
    assert!(s.add_root(&mut col, &NewRoot { frames: 30, snapshot: Some(vec![]), ..Default::default() }).is_err());

    // A blob that was there already stays when a later add fails.
    let (state, _) = s.state_at(&col, "seg", 5).unwrap();
    s.add_root(&mut col, &NewRoot { frames: 30, snapshot: Some(state.clone()), ..Default::default() }).unwrap();
    assert_eq!(blobs(&col), 1);
    assert!(s.add_root(&mut col, &NewRoot { frames: 0, snapshot: Some(state), lines: vec!["P1 JOYPAD B 1".into()], ..Default::default() }).is_err());
    assert_eq!(blobs(&col), 1);
    // A snapshot the core can no longer load (a rebuilt core, a file cut short) names the
    // segment and the file.
    let kept = col.segments.keys().find(|k| *k != "seg").unwrap().clone();
    let blob = col.segments[&kept].get("snapshot").unwrap().to_string();
    let short = b"cut".to_vec();
    std::fs::remove_file(col.blob_path(&blob, "snapshot").unwrap()).unwrap();
    let hash = col.write_blob(&short, "snapshot", "short").unwrap();
    col.segments.get_mut(&kept).unwrap().set("snapshot", &hash);
    let err = format!("{:#}", s.play_chain(&col, &kept, &PlayOptions::default(), false).err().unwrap());
    assert!(err.contains(&kept[..8]) && err.contains("blobs/short-") && err.contains("testcore 1") && err.contains("can't load"), "{err}");
    col.segments.get_mut(&kept).unwrap().set("snapshot", "0123456789abcdef0123456789abcdef");
    let err = format!("{:#}", s.play_chain(&col, &kept, &PlayOptions::default(), false).err().unwrap());
    assert!(err.contains(&kept[..8]) && err.contains("no snapshot with hash"), "{err}");
    // The session still plays after a failed load.
    assert!(s.play_chain(&col, "seg", &PlayOptions::default(), false).is_ok());
}

/// The save RAM at a frame, exported, starts a root that plays as a power-on with that save
/// in the cartridge; renaming its blob changes neither what plays nor the cache key.
fn a_root_from_exported_save_ram() {
    // A core with no save RAM says so.
    let Some((col, lib)) = project("nosave") else { return };
    {
        let mut s = Session::open_with(&col, &col.root.join("rom.bin"), &lib, false).unwrap();
        let err = s.sram_at(&col, "seg", 10).unwrap_err();
        assert!(format!("{err:#}").contains("no save RAM"), "{err:#}");
    }

    let Some((mut col, lib)) = project_with("save", &["-DSAVE_RAM"]) else { return };
    let mut s = Session::open_with(&col, &col.root.join("rom.bin"), &lib, false).unwrap();
    // B (bit 0) is held on frames 3 to 19: by frame 10 the save has counted 7 of them.
    let early = s.sram_at(&col, "seg", 10).unwrap();
    let late = s.sram_at(&col, "seg", 40).unwrap();
    assert_eq!((early.len(), early[0]), (8, 7));
    assert_ne!(early, late);
    // Loading a state brings the save RAM back with it.
    assert_eq!(s.sram_at(&col, "seg", 10).unwrap(), early);

    let plain = s.add_root(&mut col, &NewRoot { title: "plain".into(), frames: 5, sram: Some(vec![0; 8]), ..Default::default() }).unwrap();
    let id = s.add_root(&mut col, &NewRoot { title: "with save".into(), frames: 5, sram: Some(late.clone()), blob_name: "after-level-1".into(), ..Default::default() }).unwrap();
    let blob = col.segments[&id].get("sram").unwrap().to_string();
    assert_eq!(blob, dj_core::project::blob_id(&late), "the header names the content");
    let file = format!("after-level-1-{}.srm", &blob[..8]);
    assert!(col.blob_path(&blob, "sram").unwrap().ends_with(&file));
    assert_eq!(col.blob_name_of(&blob, "sram").as_deref(), Some("after-level-1"));
    let with = s.play_chain(&col, &id, &PlayOptions::default(), false).unwrap().pop().unwrap();
    assert_eq!(s.save_ram().unwrap(), late, "nothing held, so the save is as it was loaded");
    let without = s.play_chain(&col, &plain, &PlayOptions::default(), false).unwrap().pop().unwrap();
    assert_ne!(with.end_hash, without.end_hash, "the save is what the game starts with");

    // A rename moves the file only: the segment and its content key are as they were.
    let key = s.content_key(&col, &id, &with.start_hash).unwrap();
    let before = std::fs::read(&col.paths[&id]).unwrap();
    assert_eq!(col.rename_blob(&blob, "sram", "world-2").unwrap(), format!("world-2-{}", &blob[..8]));
    assert!(!col.root.join("blobs").join(&file).exists());
    assert_eq!(std::fs::read(&col.paths[&id]).unwrap(), before);
    assert_eq!(s.content_key(&col, &id, &with.start_hash).unwrap(), key);
    let same = |s: &mut Session, col: &Project| s.play_chain(col, &id, &PlayOptions::default(), false).unwrap().pop().unwrap().end_hash == with.end_hash;
    assert!(same(&mut s, &col));

    // Renamed by hand, to a name with no hash digits or to the bare hash older projects
    // use: still found, by its content.
    let blobs = col.root.join("blobs");
    std::fs::rename(blobs.join(format!("world-2-{}.srm", &blob[..8])), blobs.join("finals.srm")).unwrap();
    assert!(same(&mut s, &col));
    assert_eq!(col.blob_name_of(&blob, "sram").as_deref(), Some("finals"));
    std::fs::rename(blobs.join("finals.srm"), blobs.join(format!("{blob}.srm"))).unwrap();
    assert!(same(&mut s, &col));
    assert_eq!(col.blob_name_of(&blob, "sram"), None);
    assert_eq!(col.rename_blob(&blob, "sram", "world-2").unwrap(), format!("world-2-{}", &blob[..8]));

    // Content blobs/ has is not copied again under another name; a bad name is refused.
    assert_eq!(col.write_blob_new(&late, "sram", "again").unwrap(), (blob.clone(), None));
    assert!(col.write_blob(&early, "sram", "has space").is_err());
    let list = col.blobs();
    assert_eq!(list.iter().map(|b| (b.field, b.users.len())).collect::<Vec<_>>(), [("sram", 1), ("sram", 1)]);

    // A file that was changed is refused, with its name.
    std::fs::write(blobs.join(format!("world-2-{}.srm", &blob[..8])), &early).unwrap();
    let err = col.read_blob(&blob, "sram").unwrap_err();
    assert!(format!("{err:#}").contains("world-2") && format!("{err:#}").contains("wrong content"), "{err:#}");
    std::fs::remove_file(blobs.join(format!("world-2-{}.srm", &blob[..8]))).unwrap();
    let err = col.read_blob(&blob, "sram").unwrap_err();
    assert!(format!("{err:#}").contains("no save file with hash"), "{err:#}");
}
