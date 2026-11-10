//! The note dj-gui leaves when a core crashes or hangs it (player/src/crash.rs): the crash
//! handler writes it as the process dies, and the watchdog writes one while the GUI's frame
//! doesn't come round and removes it when it does.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn root(name: &str) -> PathBuf {
    let r = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(r.join(".local")).unwrap();
    r
}

/// Run as a child by `a_crash_leaves_a_note`: set the note up and crash in "retro_run".
#[test]
fn crashing_child() {
    let Some(dir) = std::env::var_os("DJ_TEST_CRASH_ROOT") else { return };
    decomp_dj::crash::set_note_path(std::path::Path::new(&dir));
    decomp_dj::crash::set_core("cores/broken_libretro.dylib");
    decomp_dj::crash::set_place("d0c14220-1332-4137-b40c-9a07aa53bde5", 4321);
    decomp_dj::crash::enter(1);
    unsafe { std::ptr::write_volatile(std::ptr::null_mut::<u32>(), 1) };
}

#[test]
#[cfg(unix)]
fn a_crash_leaves_a_note() {
    let dir = root("crash-note");
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crashing_child", "--nocapture", "--test-threads", "1"])
        .env("DJ_TEST_CRASH_ROOT", &dir)
        .output()
        .unwrap();
    assert!(!out.status.success(), "the child should have crashed");
    let notes: Vec<_> = std::fs::read_dir(dir.join(".local")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("crash-gui-")).collect();
    assert_eq!(notes.len(), 1);
    let text = std::fs::read_to_string(notes[0].path()).unwrap();
    assert!(text.starts_with("dj-gui crashed (signal "), "{text}");
    for want in ["core: cores/broken_libretro.dylib", "segment: d0c14220-1332-4137-b40c-9a07aa53bde5", "frame: 4321", "in: retro_run"] {
        assert!(text.contains(want), "{want} in {text}");
    }
    // The next launch finds it, as left by a process that has gone.
    let left = decomp_dj::recovery::left_behind(&dir);
    assert_eq!(left.len(), 1);
    assert!(left[0].note.as_deref() == Some(text.as_str()) && left[0].snapshot.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hang_leaves_a_note_until_the_gui_comes_back() {
    let dir = root("hang-note");
    decomp_dj::crash::set_note_path(&dir);
    decomp_dj::crash::set_core("cores/slow_libretro.dylib");
    decomp_dj::crash::set_place("seg", 7);
    decomp_dj::crash::start_watchdog(Duration::from_secs(1));
    let note = decomp_dj::crash::note_path(&dir, std::process::id());
    std::thread::sleep(Duration::from_millis(2500)); // no heartbeat: "stopped responding"
    let text = std::fs::read_to_string(&note).expect("a note while it doesn't respond");
    assert!(text.starts_with("dj-gui stopped responding for over 1 s") && text.contains("segment: seg") && text.contains("frame: 7"), "{text}");
    // Frames come round again (a GUI beats every frame): the note goes.
    for _ in 0..12 {
        decomp_dj::crash::heartbeat();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!note.exists(), "the note goes once it responds again");
    let _ = std::fs::remove_dir_all(&dir);
}
