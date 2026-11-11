//! Comparison cores end to end, with the tiny libretro core in tests/testcore (built here
//! with the system C compiler; the test is skipped when there is none).

use decomp_dj::collection::Collection;
use decomp_dj::compare::{Comparison, RemoteComparison, SyncOutcome, Verdict};
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::parity::{check_span, Checker};
use decomp_dj::player::Session;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The test core's diffs where `testcore_break = "mismatch"` goes wrong: RAM 0x02 (XOR'd with
/// 0x55), then a diff with a label alone.
fn assert_diffs(d: &decomp_dj::compare::Diffs) {
    assert_eq!(d.total, 2, "{d:?}");
    let ram = &d.list[0];
    assert_eq!((ram.label.as_deref(), ram.region.as_deref(), ram.address, ram.len), (Some("mixed state"), Some("RAM"), Some(2), 1));
    assert_eq!(ram.expected[0] ^ 0x55, ram.actual[0]);
    let only = &d.list[1];
    assert_eq!((only.label.as_deref(), only.region.as_deref(), only.address, only.expected.len()), (Some("checksum of the state"), None, None, 0));
}

fn build(out: &Path, name: &str, defs: &[&str]) -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    let lib = out.join(name);
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).args(defs).status().ok()?.success();
    ok.then_some(lib)
}

const SEGMENT: &str = "dj-segment 1\nid: seg\nparent: none\nframes: 20\n---\n2 P1 JOYPAD B 1\n6 P1 JOYPAD B 0\n8 P1 JOYPAD Y 1\n12 [WRITE] RAM 0x20 ab cd\n";

fn project(dir: &Path, core: &Path, nodj: &Path) -> Collection {
    std::fs::create_dir_all(dir.join("segments")).unwrap();
    std::fs::create_dir_all(dir.join(".local")).unwrap();
    std::fs::write(dir.join("rom.bin"), b"rom").unwrap();
    std::fs::write(dir.join("segments/seg.segment"), SEGMENT).unwrap();
    // Forward slashes: a Windows path's backslashes would be escapes in a TOML string.
    let c = core.display().to_string().replace('\\', "/");
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"1\"\npath = \"{c}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n\
             [[compare]]\nname = \"state\"\npath = \"{c}\"\nsync = \"state\"\n\
             [[compare]]\nname = \"dj\"\npath = \"{c}\"\n\
             [[compare]]\nname = \"dj-fail\"\npath = \"{c}\"\n[compare.options]\ntestcore_sync = \"fail\"\n\
             [[compare]]\nname = \"dj-mismatch\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"mismatch\"\n\
             [[compare]]\nname = \"dj-tick\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"tick\"\n\
             [[compare]]\nname = \"dj-crash\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"crash\"\n\
             [[compare]]\nname = \"dj-hang\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"hang\"\n\
             [[compare]]\nname = \"dj-crash-check\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"crash-check\"\n\
             [[compare]]\nname = \"nodj\"\npath = \"{}\"\n",
            nodj.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    Collection::open(dir).unwrap()
}

fn ram(core: &mut decomp_dj::libretro::Core) -> Vec<u8> {
    core.memory(MEMORY_SYSTEM_RAM).unwrap().to_vec()
}

/// The two tests load cores into the same process-wide slots: one at a time.
static SLOTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn comparison_cores() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    let col = project(&dir, &core, &nodj);
    let rom = dir.join("rom.bin");
    let mut s = Session::open_with(&col, &rom, &core, false).unwrap();
    s.enter_replay(&col, "seg").unwrap();

    // The same library as the baseline: loaded from a private copy, with its own state.
    let mut c = Comparison::open(&col, "state", &core, Some(&rom)).unwrap();
    for f in 0..5 {
        s.step(&col, "seg", f, false).unwrap();
    }
    assert_ne!(ram(&mut s.core), ram(&mut c.core));
    assert_eq!(c.sync(&mut s.core, &col, "seg", 5, 5, false).unwrap(), SyncOutcome::Synced);
    assert_eq!(ram(&mut s.core), ram(&mut c.core));
    for f in 5..20 {
        s.step(&col, "seg", f, false).unwrap();
        let st = c.step(&s.core, &col, "seg", f, f == 19).unwrap();
        assert!(!st.needs_sync);
        assert_eq!(ram(&mut s.core), ram(&mut c.core), "frame {f}");
    }
    assert_eq!(&ram(&mut c.core)[0x20..0x22], &[0xab, 0xcd]);
    drop(c);

    // dj_sync: told it's a boot at frame 0, then follows inputs and writes (dj_write).
    let mut s2 = {
        drop(s);
        Session::open_with(&col, &rom, &core, false).unwrap()
    };
    s2.enter_replay(&col, "seg").unwrap();
    let mut c = Comparison::open(&col, "dj", &core, Some(&rom)).unwrap();
    assert_eq!(c.sync(&mut s2.core, &col, "seg", 0, 0, true).unwrap(), SyncOutcome::Synced);
    assert_eq!(ram(&mut c.core)[63], 1);
    for f in 0..20 {
        s2.step(&col, "seg", f, false).unwrap();
        let st = c.step(&s2.core, &col, "seg", f, false).unwrap();
        assert!(st.tick_failed.is_none());
        assert_eq!(ram(&mut s2.core)[..63], ram(&mut c.core)[..63], "frame {f}");
        assert_eq!(c.check(&mut s2.core, f).unwrap(), Verdict::Match, "frame {f}");
    }
    assert!(c.has_check());
    assert_eq!(c.build.len(), 64);
    drop(c);

    // Parity fails on the frame where the core goes wrong (dj_run changes its RAM there),
    // with the core's reason; and a tick can fail with its own reason.
    for (name, tick) in [("dj-mismatch", false), ("dj-tick", true)] {
        s2.enter_replay(&col, "seg").unwrap();
        let mut c = Comparison::open(&col, name, &core, Some(&rom)).unwrap();
        assert_eq!(c.sync(&mut s2.core, &col, "seg", 0, 0, true).unwrap(), SyncOutcome::Synced);
        let mut first_bad = None;
        for f in 0..20 {
            s2.step(&col, "seg", f, false).unwrap();
            let st = c.step(&s2.core, &col, "seg", f, false).unwrap();
            if let Some(why) = st.tick_failed {
                assert!(tick && why.contains("told to fail the tick"), "{why}");
                first_bad = Some(f);
                break;
            }
            if let Verdict::Mismatch(why, diffs) = c.check(&mut s2.core, f).unwrap() {
                assert!(!tick && why.contains("RAM 0x02"), "{why}");
                assert_diffs(&diffs);
                first_bad = Some(f);
                break;
            }
        }
        assert_eq!(first_bad, Some(9), "{name}");
        drop(c);
    }

    // A sync the core declines, with its reason.
    let mut c = Comparison::open(&col, "dj-fail", &core, Some(&rom)).unwrap();
    match c.sync(&mut s2.core, &col, "seg", 20, 20, false).unwrap() {
        SyncOutcome::Failed(r) => assert!(r.contains("told to fail at frame 20"), "{r}"),
        o => panic!("{o:?}"),
    }
    drop(c);

    // sync = "dj_sync" (the default) needs the function.
    let e = Comparison::open(&col, "nodj", &core, Some(&rom)).err().expect("loading without dj_sync must fail");
    assert!(format!("{e:#}").contains("dj_sync"), "{e:#}");
    // ...and the failed load left the slot free.
    let c = Comparison::open(&col, "state", &core, Some(&rom)).unwrap();
    drop(c);
    drop(s2);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// The same comparisons with each comparison core in its own worker process, as the GUI
/// runs them (.agents/ISOLATION_PLAN.md), and cores that crash or hang there.
#[test]
fn comparison_cores_in_workers() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("DJ_WORKER_EXE", env!("CARGO_BIN_EXE_dj"));
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-workers-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    let col = project(&dir, &core, &nodj);
    let rom = dir.join("rom.bin");
    let mut s = Session::open_with(&col, &rom, &core, false).unwrap();

    // Save-state sync, inputs and [WRITE] lines followed: parity holds on every frame.
    s.enter_replay(&col, "seg").unwrap();
    let mut c = RemoteComparison::open(&col, "state", &core, Some(&rom)).unwrap();
    for f in 0..5 {
        s.step(&col, "seg", f, false).unwrap();
    }
    assert_eq!(c.sync(&mut s.core, &col, "seg", 5, 5, false).unwrap(), SyncOutcome::Synced);
    for f in 5..20 {
        s.step(&col, "seg", f, false).unwrap();
        let st = c.step(&mut s.core, &col, "seg", f, f == 19).unwrap();
        assert!(!st.needs_sync);
        assert_eq!(c.check(&mut s.core, f).unwrap(), Verdict::Match, "frame {f}");
        if f == 19 {
            let fr = st.frame.expect("a captured frame");
            assert_eq!((fr.width, fr.height, fr.rgb.len()), (4, 4, 48));
        }
    }
    drop(c);

    // dj_sync through the worker's copy of the baseline; dj_write with the baseline's state.
    s.enter_replay(&col, "seg").unwrap();
    let mut c = RemoteComparison::open(&col, "dj", &core, Some(&rom)).unwrap();
    assert!(c.has_check() && c.has_write() && c.build.len() == 64);
    assert_eq!(c.sync(&mut s.core, &col, "seg", 0, 0, true).unwrap(), SyncOutcome::Synced);
    for f in 0..20 {
        s.step(&col, "seg", f, false).unwrap();
        let st = c.step(&mut s.core, &col, "seg", f, false).unwrap();
        assert!(st.tick_failed.is_none());
        assert_eq!(c.check(&mut s.core, f).unwrap(), Verdict::Match, "frame {f}");
    }
    drop(c);

    // A mismatch and a failed tick, with the core's reasons, on the same frame as in process.
    for (name, tick) in [("dj-mismatch", false), ("dj-tick", true)] {
        s.enter_replay(&col, "seg").unwrap();
        let mut c = RemoteComparison::open(&col, name, &core, Some(&rom)).unwrap();
        assert_eq!(c.sync(&mut s.core, &col, "seg", 0, 0, true).unwrap(), SyncOutcome::Synced);
        let mut first_bad = None;
        for f in 0..20 {
            s.step(&col, "seg", f, false).unwrap();
            let st = c.step(&mut s.core, &col, "seg", f, false).unwrap();
            if let Some(why) = st.tick_failed {
                assert!(tick && why.contains("told to fail the tick"), "{why}");
                first_bad = Some(f);
                break;
            }
            if let Verdict::Mismatch(why, diffs) = c.check(&mut s.core, f).unwrap() {
                assert!(!tick && why.contains("RAM 0x02"), "{why}");
                assert_diffs(&diffs);
                first_bad = Some(f);
                break;
            }
        }
        assert_eq!(first_bad, Some(9), "{name}");
    }

    // A declined sync, and a load error, come back as before.
    let mut c = RemoteComparison::open(&col, "dj-fail", &core, Some(&rom)).unwrap();
    assert!(matches!(c.sync(&mut s.core, &col, "seg", 20, 20, false).unwrap(), SyncOutcome::Failed(r) if r.contains("told to fail at frame 20")));
    let e = RemoteComparison::open(&col, "nodj", &core, Some(&rom)).err().expect("loading without dj_sync must fail");
    assert!(format!("{e:#}").contains("dj_sync"), "{e:#}");

    // A core that crashes on frame 9, one that hangs there, and one that crashes in
    // dj_check_parity: each ends its worker with a message saying where, and dj goes on.
    std::env::set_var("DJ_WORKER_TIMEOUT_SCALE", "0.2"); // 2 s instead of 10
    for (name, expect) in [("dj-crash", "crashed"), ("dj-hang", "didn't finish"), ("dj-crash-check", "crashed")] {
        s.enter_replay(&col, "seg").unwrap();
        let mut c = RemoteComparison::open(&col, name, &core, Some(&rom)).unwrap();
        assert_eq!(c.sync(&mut s.core, &col, "seg", 0, 0, true).unwrap(), SyncOutcome::Synced);
        let started = std::time::Instant::now();
        let mut err = None;
        for f in 0..20 {
            s.step(&col, "seg", f, false).unwrap();
            if let Err(e) = c.step(&mut s.core, &col, "seg", f, false).and_then(|_| c.check(&mut s.core, f)) {
                err = Some((f, format!("{e:#}")));
                break;
            }
        }
        let (f, e) = err.unwrap_or_else(|| panic!("{name} ran to the end"));
        assert_eq!(f, 9, "{name}: {e}");
        assert!(e.contains(expect) && e.contains("Restart comparison core"), "{name}: {e}");
        let place = if name == "dj-crash-check" { "dj_check_parity" } else { "running frame 9" };
        assert!(e.contains(place), "{name}: {e}");
        assert!(c.dead().is_some());
        assert!(started.elapsed() < std::time::Duration::from_secs(8), "{name}: {:?}", started.elapsed());
        // Once stopped, it says so; the baseline plays on.
        let again = format!("{:#}", c.sync(&mut s.core, &col, "seg", 0, 0, true).unwrap_err());
        assert!(again.contains("has stopped"), "{again}");
        s.step(&col, "seg", 10, false).unwrap();
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `parity::check_span`, as `dj parity` and dj-gui use it: syncs, checks each frame, and
/// stops at the first failure with how to reproduce it.
#[test]
fn checking_spans() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("DJ_WORKER_EXE", env!("CARGO_BIN_EXE_dj"));
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-spans-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    let col = project(&dir, &core, &nodj);
    let rom = dir.join("rom.bin");
    let mut s = Session::open_with(&col, &rom, &core, false).unwrap();

    // Every frame passes, [WRITE] included (dj_write).
    s.enter_replay(&col, "seg").unwrap();
    let mut c = RemoteComparison::open(&col, "dj", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 0, 20, 0).unwrap();
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, span.skipped, span.unchecked), (20, 0, 0));
    drop(c);

    // Started partway (save-state sync at frame 5): the frames before aren't checked.
    s.enter_replay(&col, "seg").unwrap();
    for f in 0..5 {
        s.step(&col, "seg", f, false).unwrap();
    }
    let mut c = RemoteComparison::open(&col, "state", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 5, 20, 0).unwrap();
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!(span.passed, 15);
    drop(c);

    // A mismatch at frame 9: the frames before passed, and the failure says where the run
    // was synced, so it can be re-run.
    s.enter_replay(&col, "seg").unwrap();
    let mut c = RemoteComparison::open(&col, "dj-mismatch", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 0, 20, 0).unwrap();
    let f = span.failure.expect("a failure");
    assert_eq!((f.segment.as_str(), f.frame, f.synced.clone()), ("seg", 9, ("seg".to_string(), 0)));
    assert!(f.reason.contains("Parity failure at frame 9") && f.reason.contains("RAM 0x02"), "{}", f.reason);
    assert_eq!((f.core.as_str(), f.build.as_str()), ("dj-mismatch", c.build.as_str()));
    assert_eq!(span.passed, 9);
    drop(c);

    // A sync the core declines is a failure at the frame it was tried.
    s.enter_replay(&col, "seg").unwrap();
    let mut c = RemoteComparison::open(&col, "dj-fail", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 0, 20, 0).unwrap();
    let f = span.failure.expect("a failed sync");
    assert_eq!(f.frame, 0);
    assert!(f.reason.contains("Couldn't sync at frame 0") && f.reason.contains("told to fail"), "{}", f.reason);
    assert_eq!(span.passed, 0);
    drop(c);

    drop(s);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `dj parity` end to end: checks, skips what it checked, fails with exit 1, and reports,
/// re-runs and clears failures.
#[test]
fn dj_parity_command() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-cmd-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    project(&dir, &core, &nodj);
    let dj = |args: &[&str]| -> (i32, String, String) {
        // `dj <command> <dir> [options]`: the folder first, so a list option can't take it.
        let out = Command::new(env!("CARGO_BIN_EXE_dj")).arg(args[0]).arg(&dir).args(&args[1..]).arg("--rom").arg(dir.join("rom.bin")).arg("--core").arg(&core).output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (code, out, err) = dj(&["clean"]);
    assert_eq!(code, 0, "{out}{err}");

    // Several comparison cores: one must be picked.
    let (code, _, err) = dj(&["parity"]);
    assert_eq!(code, 2);
    assert!(err.contains("--compare"), "{err}");

    // Every frame passes; checked again only with --all.
    let (code, out, err) = dj(&["parity", "--compare", "dj"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("passed") && out.contains("20 passed") && out.contains("0 failed"), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj"]);
    assert_eq!(code, 0);
    assert!(out.contains("checked with this build before") && out.contains("0 segment(s) checked"), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj", "--all"]);
    assert_eq!(code, 0);
    assert!(out.contains("1 segment(s) checked"), "{out}");

    // A mismatch at frame 9: exit 1, kept, and still failing when not checked again.
    let (code, out, err) = dj(&["parity", "--compare", "dj-mismatch"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("FAILED") && out.contains("frame 9") && out.contains("RAM 0x02"), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj-mismatch"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("failed with this build before"), "{out}");

    // The report lists it; JSON gives its id.
    let (code, out, err) = dj(&["parity", "--compare", "dj-mismatch", "--report", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let fs = v["failures"].as_array().unwrap();
    assert_eq!(fs.len(), 1, "{out}");
    assert_eq!((fs[0]["frame"].as_u64(), fs[0]["current_build"].as_bool()), (Some(9), Some(true)));
    let id = fs[0]["id"].as_i64().unwrap().to_string();
    // What differed is kept with the failure (dj_parity_diffs).
    assert_eq!(fs[0]["diffs_total"].as_u64(), Some(2), "{out}");
    assert_eq!((fs[0]["diffs"][0]["label"].as_str(), fs[0]["diffs"][0]["address"].as_str()), (Some("mixed state"), Some("0x0002")), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj-mismatch", "--report"]);
    assert_eq!(code, 0);
    assert!(out.contains(&format!("#{id} ")) && out.contains("mixed state (RAM 0x0002)"), "{out}");

    // --explain plays it again: the test core has no trace hook, so each diff says why it
    // wasn't traced.
    let (code, out, err) = dj(&["parity", "--compare", "dj-mismatch", "--explain", &id, "--json"]);
    assert_eq!(code, 1, "{out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!((v["outcome"].as_str(), v["frame"].as_u64(), v["diffs_total"].as_u64()), (Some("failed"), Some(9), Some(2)), "{out}");
    assert!(v["traced"].is_null(), "{out}");
    assert!(v["diffs"][0]["note"].as_str().unwrap().contains("no trace hook"), "{out}");
    assert!(v["diffs"][1]["note"].as_str().unwrap().contains("no address"), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj-mismatch", "--explain", &id]);
    assert_eq!(code, 1);
    assert!(out.contains("fails again") && out.contains("mixed state (RAM 0x0002)"), "{out}");

    // Re-run: it fails again (and stays a single failure); with no ids, only older builds'.
    let (code, out, err) = dj(&["parity", "--compare", "dj-mismatch", "--rerun", &id]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("fails again"), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj-mismatch", "--rerun"]);
    assert_eq!(code, 0);
    assert!(out.contains("no failures from an older build"), "{out}");
    let (_, out, _) = dj(&["parity", "--compare", "dj-mismatch", "--report", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let fs = v["failures"].as_array().unwrap();
    assert_eq!(fs.len(), 1, "{out}");

    // Clear it.
    let id = fs[0]["id"].as_i64().unwrap().to_string();
    let (code, out, err) = dj(&["parity", "--compare", "dj-mismatch", "--clear", &id]);
    assert_eq!(code, 0, "{out}{err}");
    let (_, out, _) = dj(&["parity", "--compare", "dj-mismatch", "--report"]);
    assert!(out.contains("no failures kept"), "{out}");

    // A comparison core that crashes: a failure that says so, and dj carries on to report it.
    std::env::set_var("DJ_WORKER_TIMEOUT_SCALE", "0.2");
    let (code, out, err) = dj(&["parity", "--compare", "dj-crash"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("FAILED") && out.contains("crashed"), "{out}");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// The MCP parity tools: a parity_check job, then parity_summary and parity_failures, and a
/// parity_rerun job.
#[test]
fn mcp_parity_tools() {
    use std::io::{BufRead, BufReader, Write};
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    project(&dir, &core, &nodj);
    // Clean (which also remembers the ROM and core for this machine).
    let st = Command::new(env!("CARGO_BIN_EXE_dj")).arg("clean").arg(&dir).arg("--rom").arg(dir.join("rom.bin")).arg("--core").arg(&core).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));

    let mut child = Command::new(env!("CARGO_BIN_EXE_dj"))
        .arg("mcp")
        .arg(&dir)
        .env("DJ_WORKER_EXE", env!("CARGO_BIN_EXE_dj"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut next = 0;
    let mut call = |tool: &str, args: serde_json::Value| -> serde_json::Value {
        next += 1;
        let req = serde_json::json!({"jsonrpc": "2.0", "id": next, "method": "tools/call", "params": {"name": tool, "arguments": args}});
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let r = &v["result"];
        assert!(!r["isError"].as_bool().unwrap_or(false), "{tool}: {r}");
        serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let wait = |call: &mut dyn FnMut(&str, serde_json::Value) -> serde_json::Value, job: &str| -> serde_json::Value {
        for _ in 0..600 {
            let s = call("job_status", serde_json::json!({"job": job}));
            if !matches!(s["status"].as_str(), Some("starting" | "running")) {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("job {job} didn't finish");
    };

    // A check that fails at frame 9.
    let j = call("parity_check", serde_json::json!({"compare": "dj-mismatch"}));
    let s = wait(&mut call, j["job"].as_str().unwrap());
    assert!(s["status"].as_str().unwrap().contains("frames failed"), "{s}");
    assert_eq!(s["failed"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["frames"].as_u64(), Some(10), "{s}");

    // What the queries say about it.
    let sum = call("parity_summary", serde_json::json!({"compare": "dj-mismatch"}));
    let fs = sum["failures"].as_array().unwrap();
    assert_eq!(fs.len(), 1, "{sum}");
    assert_eq!((fs[0]["frame"].as_u64(), fs[0]["current_build"].as_bool()), (Some(9), Some(true)));
    let f = call("parity_failures", serde_json::json!({"compare": "dj-mismatch", "old": true}));
    assert_eq!(f["total"].as_u64(), Some(0), "{f}");
    let f = call("parity_failures", serde_json::json!({"compare": "dj-mismatch", "old": false}));
    assert_eq!(f["total"].as_u64(), Some(1), "{f}");
    let id = f["failures"][0]["id"].as_i64().unwrap();
    assert_eq!(f["failures"][0]["diffs"][0]["text"].as_str().map(|t| t.starts_with("mixed state (RAM 0x0002): ")), Some(true), "{f}");

    // Explain it: it fails again, with what differed.
    let e = call("parity_explain", serde_json::json!({"compare": "dj-mismatch", "id": id}));
    assert_eq!((e["outcome"].as_str(), e["diffs_total"].as_u64()), (Some("failed"), Some(2)), "{e}");

    // Re-run it: it fails again.
    let j = call("parity_rerun", serde_json::json!({"compare": "dj-mismatch", "ids": [id]}));
    let s = wait(&mut call, j["job"].as_str().unwrap());
    assert!(s["status"].as_str().unwrap().contains("frames failed"), "{s}");
    assert_eq!(s["results"][0]["verdict"].as_str(), Some("failed"), "{s}");

    // A core that matches: the job finishes with nothing failed.
    let j = call("parity_check", serde_json::json!({"compare": "dj"}));
    let s = wait(&mut call, j["job"].as_str().unwrap());
    assert_eq!(s["status"].as_str(), Some("finished: nothing failed"), "{s}");
    assert_eq!(s["totals"]["checked"].as_u64(), Some(1), "{s}");

    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A copy of a core loaded before the original: on macOS, a library whose install name is its
/// absolute path (as `cc -o /abs/path` builds the test core) is handed back for the original,
/// so the two slots would share one instance. dj refuses that before calling into it.
#[test]
fn a_shared_library_instance_is_refused() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-shared-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let Some(core) = build(&tmp, "testcore_shared.so", &[]) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let copy = tmp.join("copy-testcore_shared.so");
    std::fs::copy(&core, &copy).unwrap();
    let opts = Default::default();
    let first = decomp_dj::libretro::Core::load_in(decomp_dj::libretro::COMPARISON, &copy, &tmp, &opts).unwrap();
    let second = decomp_dj::libretro::Core::load_in(decomp_dj::libretro::BASELINE, &core, &tmp, &opts);
    if cfg!(target_os = "macos") {
        let e = format!("{:#}", second.err().expect("the original must be refused"));
        assert!(e.contains("same loaded library"), "{e}");
    } else if let Err(e) = second {
        // Other systems keep the two apart; refusing is only right if they didn't.
        assert!(format!("{e:#}").contains("same loaded library"), "{e:#}");
    }
    drop(first);
    let _ = std::fs::remove_dir_all(&tmp);
}
