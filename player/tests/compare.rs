//! Comparison cores end to end, with the tiny libretro core in tests/testcore (built here
//! with the system C compiler; the test is skipped when there is none).

use decomp_dj::collection::Collection;
use decomp_dj::compare::{Comparison, RemoteComparison, SyncOutcome, Verdict};
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::player::Session;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let c = core.display();
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
            nodj.display()
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
            if let Verdict::Mismatch(why) = c.check(&mut s2.core, f).unwrap() {
                assert!(!tick && why.contains("RAM 0x02"), "{why}");
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
            if let Verdict::Mismatch(why) = c.check(&mut s.core, f).unwrap() {
                assert!(!tick && why.contains("RAM 0x02"), "{why}");
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
