//! Batch commands (`dj clean`, `dj run`) play in a child dj under a supervisor
//! (.agents/ISOLATION_PLAN.md, step 3): a core that crashes or hangs in one segment costs
//! that segment and the ones below it, and the command goes on with the rest.

use std::path::{Path, PathBuf};
use std::process::Command;

fn build_core(out: &Path) -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    let lib = out.join("testcore.so");
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).status().ok()?.success();
    ok.then_some(lib)
}

const ROOT: &str = "10000000-0000-0000-0000-000000000000";
const CRASH: &str = "20000000-0000-0000-0000-000000000000";
const BELOW: &str = "21000000-0000-0000-0000-000000000000";
const FINE: &str = "30000000-0000-0000-0000-000000000000";
const HANG: &str = "40000000-0000-0000-0000-000000000000";

fn segment(dir: &Path, id: &str, parent: &str, title: &str, frames: u64, lines: &str) {
    std::fs::write(dir.join(format!("segments/{id}.segment")), format!("dj-replay 1\nid: {id}\nparent: {parent}\ntitle: {title}\nframes: {frames}\n---\n{lines}")).unwrap();
}

fn dj(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dj"))
        .args(args)
        .arg(dir)
        .env("TESTCORE_CRASH_ON_SELECT", "1")
        .env("TESTCORE_HANG_ON_START", "1")
        .env("DJ_WORKER_TIMEOUT_SCALE", "0.1") // a hang is noticed after 3 s, not 30
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn a_segment_that_crashes_or_hangs_the_core_is_left_out() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("project/segments")).unwrap();
    std::fs::create_dir_all(tmp.join("project/.local")).unwrap();
    let Some(core) = build_core(&tmp) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    std::fs::write(dir.join("rom.bin"), b"rom").unwrap();
    std::fs::write(
        dir.join("config.toml"),
        format!("format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"1\"\npath = \"{}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n", core.display()),
    )
    .unwrap();
    segment(&dir, ROOT, "none", "boot", 10, "");
    segment(&dir, CRASH, ROOT, "crashes", 10, "3 P1 JOYPAD SELECT 1\n");
    segment(&dir, BELOW, CRASH, "below the crash", 5, "");
    segment(&dir, FINE, ROOT, "fine", 10, "2 P1 JOYPAD B 1\n");
    segment(&dir, HANG, ROOT, "hangs", 10, "4 P1 JOYPAD START 1\n");

    let started = std::time::Instant::now();
    let (code, out, err) = dj(&dir, &["clean"]);
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert!(err.contains("crashed  20000000 crashes at frame 3: the core crashed"), "{err}");
    assert!(err.contains("crashed  40000000 hangs at frame 4: the core made no progress"), "{err}");
    assert!(err.contains("2 segment(s) crashed or hung the core"), "{err}");
    assert!(out.contains("blocked") && out.contains("21000000"), "{out}");
    assert!(started.elapsed() < std::time::Duration::from_secs(30), "{:?}", started.elapsed());
    // The rest were cleaned: their hashes are in their files; the left-out ones have none.
    let text = |id: &str| std::fs::read_to_string(dir.join(format!("segments/{id}.segment"))).unwrap();
    assert!(text(ROOT).contains("end-hash") && text(FINE).contains("end-hash"));
    assert!(!text(CRASH).contains("end-hash") && !text(BELOW).contains("end-hash") && !text(HANG).contains("end-hash"));

    // dj run: the same, reporting the clean segments as ok.
    let (code, out, err) = dj(&dir, &["run"]);
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("ok") && out.contains("30000000 fine"), "{out}");
    assert!(err.contains("crashed  20000000") && err.contains("crashed  40000000"), "{err}");

    let _ = std::fs::remove_dir_all(&tmp);
}
