//! Batch commands (`dj clean`, `dj check`) play in a child dj under a supervisor
//! (.agents/_archive/ISOLATION_PLAN.md, step 3): a core that crashes or hangs in one segment costs
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
    std::fs::write(dir.join(format!("segments/{id}.segment")), format!("dj-segment 1\nid: {id}\nparent: {parent}\ntitle: {title}\nframes: {frames}\n---\n{lines}")).unwrap();
}

fn dj(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dj")).env("NO_COLOR", "1")
        .args(args)
        .arg("-C").arg(dir)
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
        format!("format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"1\"\npath = \"{}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n", core.display().to_string().replace('\\', "/")),
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

    // dj check: the same, reporting the clean segments as ok.
    let (code, out, err) = dj(&dir, &["check"]);
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("ok") && out.contains("30000000 fine"), "{out}");
    assert!(err.contains("crashed  20000000") && err.contains("crashed  40000000"), "{err}");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// `dj upgrade` rewrites older segment files; `dj repin` records the core's build once every
/// segment still matches, and refuses while one doesn't.
#[test]
fn upgrade_and_repin() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("repin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("project/segments")).unwrap();
    let Some(core) = build_core(&tmp) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    std::fs::write(dir.join("rom.bin"), b"rom").unwrap();
    let config = |build: &str| format!("format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"{build}\"\npath = \"{}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n", core.display().to_string().replace('\\', "/"));
    std::fs::write(dir.join("config.toml"), config("testcore 1")).unwrap();
    std::fs::write(dir.join(format!("segments/{ROOT}.segment")), format!("dj-replay 1\nid: {ROOT}\nparent: none\ntitle: boot\nframes: 10\n---\n2 P1 JOYPAD B 1\n")).unwrap();
    let (code, out, err) = dj(&dir, &["clean"]);
    assert_eq!(code, 0, "{out}{err}");

    let (code, out, _) = dj(&dir, &["upgrade"]);
    assert!(code == 0 && out.contains("0 of 1") || out.contains("1 of 1"), "{out}");
    let seg = std::fs::read_to_string(dir.join(format!("segments/{ROOT}.segment"))).unwrap();
    assert!(seg.starts_with("dj-segment 1\n"), "{seg}");
    assert!(std::fs::read_to_string(dir.join("config.toml")).unwrap().contains("dj = \""));

    // Another build recorded: warned about, then repinned to what the core reports.
    let cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap().replace("build = \"testcore 1\"", "build = \"testcore 0\"");
    std::fs::write(dir.join("config.toml"), &cfg).unwrap();
    let (code, out, err) = dj(&dir, &["repin"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(err.contains("reports build \"testcore 1\"") && out.contains("\"testcore 0\" -> \"testcore 1\""), "{out}{err}");
    assert!(std::fs::read_to_string(dir.join("config.toml")).unwrap().contains("build = \"testcore 1\""));

    // A segment that no longer reproduces: nothing is recorded.
    let seg = std::fs::read_to_string(dir.join(format!("segments/{ROOT}.segment"))).unwrap();
    let broken = seg.lines().map(|l| if l.starts_with("end-hash:") { "end-hash: 00000000000000000000000000000000".to_string() } else { l.to_string() }).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(dir.join(format!("segments/{ROOT}.segment")), broken).unwrap();
    std::fs::write(dir.join("config.toml"), cfg).unwrap();
    let (code, out, err) = dj(&dir, &["repin"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(err.contains("not repinned"), "{err}");
    assert!(std::fs::read_to_string(dir.join("config.toml")).unwrap().contains("build = \"testcore 0\""));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// One vocabulary for the commands that play: `clean SEGMENT` cleans that segment alone and
/// `--children` those below it, `cache fill` fills the database and writes no file,
/// `check` writes none either (of every segment or those chosen), and a `[WRITE]` at an instruction that doesn't run
/// is counted at the end.
#[test]
fn clean_check_and_the_queries() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("vocab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("project/segments")).unwrap();
    std::fs::create_dir_all(tmp.join("project/.local")).unwrap();
    // With a connector, so that there is coverage to report and an instruction to write at.
    let core = tmp.join("testcore_conn.so");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    if !Command::new("cc").args(["-shared", "-fPIC", "-O1", "-DCONNECTOR", "-o"]).arg(&core).arg(&src).status().is_ok_and(|s| s.success()) {
        eprintln!("skipped: no C compiler");
        return;
    }
    let dir = tmp.join("project");
    std::fs::write(dir.join("rom.bin"), b"rom").unwrap();
    std::fs::write(
        dir.join("config.toml"),
        format!("format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"testcore+dj\"\npath = \"{}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n", core.display().to_string().replace('\\', "/")),
    )
    .unwrap();
    segment(&dir, ROOT, "none", "boot", 10, "");
    segment(&dir, FINE, ROOT, "fine", 10, "2 P1 JOYPAD B 1\n");
    let text = |id: &str| std::fs::read_to_string(dir.join(format!("segments/{id}.segment"))).unwrap();
    // `dj <command words> -C <dir> [more]`.
    let run = |args: &[&str]| -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_dj")).env("NO_COLOR", "1").args(args[0].split(' ')).arg("-C").arg(&dir).args(&args[1..]).output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };

    // The project is found from the current folder up, or named with -C; naming it the old
    // way, as an argument, is refused with a hint.
    let bare = |args: &[&str], cwd: &Path| -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_dj")).env("NO_COLOR", "1").args(args).current_dir(cwd).output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (code, out, err) = bare(&["status"], &dir.join("segments"));
    assert!(code == 0 && out.contains("2 segments"), "{out}{err}");
    let (code, _, err) = bare(&["status"], &tmp);
    assert!(code == 2 && err.contains("not in a dj project"), "{err}");
    let (code, _, err) = bare(&["status", dir.to_str().unwrap()], &tmp);
    assert!(code == 2 && err.contains("no longer takes as an argument") && err.contains("-C"), "{err}");
    let (code, _, err) = bare(&["-C", tmp.to_str().unwrap(), "status"], &tmp);
    assert!(code == 2 && err.contains("isn't a dj project"), "{err}");
    // cache fill plays both into the database, and neither file gets a hash.
    let (code, out, err) = run(&["cache fill"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("2 played"), "{out}");
    assert!(!text(ROOT).contains("end-hash") && !text(FINE).contains("end-hash"));
    // check compares, and writes nothing either: both segments are new, which
    // passes with a note, and fails with --strict.
    let (code, out, err) = run(&["check"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("new") && !text(ROOT).contains("end-hash"), "{out}");
    assert!(err.contains("2 segment(s) have no recorded hashes"), "{err}");
    let (code, _, err) = run(&["check", "--strict"]);
    assert!(code == 1 && err.contains("failed: 2 segment(s) have no recorded hashes"), "{err}");
    // One segment: it alone is reported, played from the stored state above it.
    let (code, out, err) = run(&["check", FINE]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("30000000 fine") && !out.contains("10000000 boot") && !text(FINE).contains("end-hash"), "{out}");
    let (code, _, err) = run(&["check", "--children"]);
    assert_eq!(code, 2, "--children needs a segment or a tag: {err}");
    // A segment alone: the one below it is not touched. --children takes it too.
    let (code, out, err) = run(&["clean", ROOT]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(text(ROOT).contains("end-hash") && !text(FINE).contains("end-hash"), "{out}");
    let (code, out, err) = run(&["clean", ROOT, "--children"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(text(FINE).contains("end-hash"), "{out}");
    let (code, _, err) = run(&["clean", "--children"]);
    assert_eq!(code, 2, "--children needs a segment: {err}");
    // Stored and clean: nothing plays, unless --force.
    let (_, out, _) = run(&["clean"]);
    assert!(out.contains("0 frames"), "{out}");
    let (_, out, _) = run(&["clean", "--force"]);
    assert!(out.contains("20 frames"), "{out}");
    let (code, out, err) = run(&["check", "--from-power-on", "--no-db", "--strict"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("ok") && !out.contains("new"), "{out}");

    // The summary says what was left out, and with --segments each segment's share.
    let (code, out, err) = run(&["summary", "--segments", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!((v["segments"].as_u64(), v["segments_left_out"]["unsaved"].as_u64(), v["per_segment"].as_array().map(|a| a.len())), (Some(2), Some(0), Some(2)), "{out}");
    assert!(v["smallest_set"].is_array(), "{out}");
    let (_, out, _) = run(&["summary", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.get("per_segment").is_none() && v.get("smallest_set").is_none(), "{out}");
    let (_, out, _) = run(&["summary", "--segments"]);
    assert!(out.contains("smallest set keeping all coverage") && out.contains("10000000 boot"), "{out}");
    // A CSV of uncovered blocks needs functions, and this project has no symbol map.
    let csv = tmp.join("gaps.csv");
    let (code, _, err) = run(&["gaps", "--csv", csv.to_str().unwrap()]);
    assert!(code == 2 && err.contains("symbol map") && !csv.exists(), "{err}");

    // A write at an instruction that doesn't run in its frame: warned of, and counted at the end.
    segment(&dir, HANG, ROOT, "a write nothing reaches", 5, "1 [WRITE] RAM 0x01 77 @PRG:0xEE\n");
    let (code, out, err) = run(&["check", HANG]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(err.contains("the write was not made") && err.contains("1 [WRITE] line at an instruction not made"), "{err}");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// What CI needs: the ROM and core from DJ_ROM and DJ_CORE (and not saved), JUnit reports,
/// and states exported on one machine and seeded on another, checked by hash.
#[test]
fn ci_commands() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ci-{}", std::process::id()));
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
        format!("format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"1\"\npath = \"{}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n", core.display().to_string().replace('\\', "/")),
    )
    .unwrap();
    segment(&dir, ROOT, "none", "boot", 10, "");
    segment(&dir, FINE, ROOT, "fine", 10, "2 P1 JOYPAD B 1\n");
    // `dj <words> <dir> <more>`, with environment variables.
    let run = |words: &[&str], more: &[&str], env: &[(&str, &str)]| -> (i32, String, String) {
        let mut c = Command::new(env!("CARGO_BIN_EXE_dj"));
        c.env("NO_COLOR", "1").env_remove("DJ_ROM").env_remove("DJ_CORE").env_remove("DJ_JUNIT").args(words).arg("-C").arg(&dir).args(more);
        for (k, v) in env {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let read = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();

    // DJ_CORE wins over the settings, and isn't saved into them.
    let (code, _, err) = run(&["check"], &[], &[("DJ_CORE", "/nowhere/core")]);
    assert!(code == 2 && err.contains("/nowhere/core"), "{err}");
    assert!(!read(&dir.join(".local/local.toml")).contains("nowhere"));

    // A JUnit report from dj check, into the folder DJ_JUNIT names: nothing is recorded yet,
    // so both are skipped, or with --strict both fail.
    let reports = tmp.join("reports");
    let (code, out, err) = run(&["check"], &[], &[("DJ_JUNIT", reports.to_str().unwrap())]);
    assert_eq!(code, 0, "{out}{err}");
    let x = read(&reports.join("dj-check.xml"));
    assert!(x.contains(r#"tests="2" failures="0" errors="0" skipped="2""#) && x.contains("segments/10000000"), "{x}");
    let file = tmp.join("strict.xml");
    let (code, _, _) = run(&["check"], &["--strict", "--junit", file.to_str().unwrap()], &[]);
    assert_eq!(code, 1);
    assert!(read(&file).contains(r#"tests="2" failures="2""#), "{}", read(&file));
    let (code, out, err) = run(&["clean"], &[], &[]);
    assert_eq!(code, 0, "{out}{err}");
    let (code, _, _) = run(&["check"], &["--strict", "--junit", file.to_str().unwrap()], &[]);
    assert_eq!(code, 0);
    assert!(read(&file).contains(r#"tests="2" failures="0" errors="0" skipped="0""#), "{}", read(&file));

    // Export what checking the child starts from: the root's end, named by its hash. A root
    // alone needs nothing.
    let states = tmp.join("states");
    let (code, out, err) = run(&["cache", "export"], &[FINE, "--out", states.to_str().unwrap()], &[]);
    assert_eq!(code, 0, "{out}{err}");
    let end = read(&dir.join(format!("segments/{ROOT}.segment"))).lines().find_map(|l| l.strip_prefix("end-hash: ").map(String::from)).unwrap();
    assert!(states.join(format!("{end}.state")).is_file(), "{out}");
    let (code, out, _) = run(&["cache", "export"], &[ROOT, "--out", tmp.join("none").to_str().unwrap()], &[]);
    assert!(code == 0 && out.contains("0 state(s)"), "{out}");

    // Another machine: no database. Seeding stores it; a file that isn't a state is refused.
    for f in ["cache.db", "cache.db-wal", "cache.db-shm"] {
        let _ = std::fs::remove_file(dir.join(".local").join(f));
    }
    std::fs::write(states.join("junk.state"), b"junk").unwrap();
    let (code, out, err) = run(&["cache", "seed"], &[states.to_str().unwrap()], &[]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("1 stored, 1 refused") && out.contains(&format!("{end}  end of 10000000 boot")) && out.contains("junk.state: the core can't load it"), "{out}");
    let (_, out, _) = run(&["sql"], &["SELECT count(*) FROM snapshots"], &[]);
    assert_eq!(out.lines().nth(1), Some("1"), "{out}");
    std::fs::remove_file(states.join("junk.state")).unwrap();
    let (code, _, _) = run(&["cache", "seed"], &[states.to_str().unwrap()], &[]);
    assert_eq!(code, 0);

    // The seeded state is what the child starts from: the root is not played again. With
    // TESTCORE_CRASH_ON_SELECT the test core crashes while SELECT is held, which the root
    // does at its frame 3, so playing it again would end the command.
    let seg = |id: &str| dir.join(format!("segments/{id}.segment"));
    let root = read(&seg(ROOT));
    let crashing = root.replace("---\n", "---\n3 P1 JOYPAD SELECT 1\n5 P1 JOYPAD SELECT 0\n");
    std::fs::write(&seg(ROOT), &crashing).unwrap();
    let (code, out, err) = run(&["check"], &[FINE], &[("TESTCORE_CRASH_ON_SELECT", "1")]);
    assert_eq!(code, 0, "the child should start from the seeded state: {out}{err}");
    assert!(out.contains("ok") && out.contains("30000000 fine") && !out.contains("10000000"), "{out}");
    // Played from power-on instead, the root runs and the core crashes: the trap is real.
    // (A crash in a checked chain is a failure, exit 1, not the end of dj.)
    let (code, out, _) = run(&["check"], &[FINE, "--from-power-on"], &[("TESTCORE_CRASH_ON_SELECT", "1")]);
    assert_eq!(code, 1, "{out}");
    std::fs::write(&seg(ROOT), &root).unwrap();

    let _ = std::fs::remove_dir_all(&tmp);
}
