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
             [[compare]]\nname = \"dj-pre\"\npath = \"{c}\"\npreroll = 5\n\
             [[compare]]\nname = \"dj-halt-pre\"\npath = \"{c}\"\npreroll = 12\n[compare.options]\ntestcore_break = \"halt\"\n\
             [[compare]]\nname = \"dj-crash\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"crash\"\n\
             [[compare]]\nname = \"dj-hang\"\npath = \"{c}\"\n[compare.options]\ntestcore_break = \"hang\"\n\
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
    assert!(!c.takes_events());
    assert_eq!(c.sync(&mut s.core, &col, "seg", 5).unwrap(), SyncOutcome::Synced);
    assert_eq!(ram(&mut s.core), ram(&mut c.core));
    for f in 5..20 {
        s.step(&col, "seg", f, false).unwrap();
        c.step(&col, "seg", f, f == 19).unwrap();
        assert_eq!(ram(&mut s.core), ram(&mut c.core), "frame {f}");
        assert_eq!(c.check(&mut s.core).unwrap(), Verdict::Match, "frame {f}");
    }
    assert_eq!(&ram(&mut c.core)[0x20..0x22], &[0xab, 0xcd]);
    drop(c);

    // The default, sync = "events": the core takes events and is not synced or run by dj
    // (tests/events.rs drives it).
    let mut s2 = {
        drop(s);
        Session::open_with(&col, &rom, &core, false).unwrap()
    };
    s2.enter_replay(&col, "seg").unwrap();
    let mut c = Comparison::open(&col, "dj", &core, Some(&rom)).unwrap();
    assert!(c.takes_events() && c.handles_writes() && c.breakpoints().is_empty());
    assert_eq!(c.build.len(), 64);
    assert!(c.sync(&mut s2.core, &col, "seg", 0).is_err());
    assert!(c.step(&col, "seg", 0, false).is_err());
    drop(c);

    // It needs dj_handle_event.
    let e = Comparison::open(&col, "nodj", &core, Some(&rom)).err().expect("loading without dj_handle_event must fail");
    assert!(format!("{e:#}").contains("dj_handle_event"), "{e:#}");
    // ...and the failed load left the slot free.
    let c = Comparison::open(&col, "state", &core, Some(&rom)).unwrap();
    drop(c);
    drop(s2);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A `sync = "state"` comparison core in its own worker process (.agents/ISOLATION_PLAN.md).
/// dj doesn't run them this way; the worker is kept so that going back is cheap. A core that
/// takes events can't run in one.
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
    assert_eq!(c.sync(&mut s.core, &col, "seg", 5).unwrap(), SyncOutcome::Synced);
    for f in 5..20 {
        s.step(&col, "seg", f, false).unwrap();
        let st = c.step(&col, "seg", f, f == 19).unwrap();
        assert_eq!(c.check(&mut s.core).unwrap(), Verdict::Match, "frame {f}");
        if f == 19 {
            let fr = st.frame.expect("a captured frame");
            assert_eq!((fr.width, fr.height, fr.rgb.len()), (4, 4, 48));
        }
    }
    drop(c);

    // A core that takes events is refused, and so is one with no dj_handle_event.
    let e = RemoteComparison::open(&col, "dj", &core, Some(&rom)).err().expect("an event core in a worker must be refused");
    assert!(format!("{e:#}").contains("takes events"), "{e:#}");
    drop(s);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `parity::check_span`, as `dj parity` and dj-gui use it: syncs, checks each frame, and
/// stops at the first failure with how to reproduce it.
#[test]
fn checking_spans() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
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

    // Every frame passes, [WRITE] included (dj_handle_write).
    s.enter_replay(&col, "seg").unwrap();
    let mut c = Comparison::open(&col, "dj", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 0, 20, 0).unwrap();
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, span.skipped, span.unchecked), (20, 0, 0));
    drop(c);

    // Started partway (save-state sync at frame 5): the frames before aren't checked.
    s.enter_replay(&col, "seg").unwrap();
    for f in 0..5 {
        s.step(&col, "seg", f, false).unwrap();
    }
    let mut c = Comparison::open(&col, "state", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 5, 20, 0).unwrap();
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!(span.passed, 15);
    drop(c);

    // A mismatch at frame 9: the frames before passed, and the failure says where the run
    // was synced, so it can be re-run.
    s.enter_replay(&col, "seg").unwrap();
    let mut c = Comparison::open(&col, "dj-mismatch", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 0, 20, 0).unwrap();
    let f = span.failure.expect("a failure");
    assert_eq!((f.segment.as_str(), f.frame, f.synced.clone()), ("seg", 9, ("seg".to_string(), 0)));
    assert!(f.reason.contains("Parity failure at frame 9") && f.reason.contains("RAM 0x02"), "{}", f.reason);
    assert_eq!((f.core.as_str(), f.build.as_str()), ("dj-mismatch", c.build.as_str()));
    assert_eq!(span.passed, 9);
    drop(c);

    // A core that can't copy the baseline judges nothing: no failure, and no frame passes.
    s.enter_replay(&col, "seg").unwrap();
    let mut c = Comparison::open(&col, "dj-fail", &core, Some(&rom)).unwrap();
    let span = check_span(&mut s, &mut c, &mut Checker::default(), &col, "seg", 0, 20, 0).unwrap();
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, span.skipped), (0, 20));
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

    // Pre-roll ([[compare]] preroll): a child is checked after playing the parent's last
    // frames, which count as nothing. A core that halts in them leaves the child not checked,
    // with the failure at the parent's frame.
    std::fs::write(dir.join("segments/kid.segment"), "dj-segment 1\nid: kid\nparent: seg\nframes: 10\n---\n3 P1 JOYPAD A 1\n").unwrap();
    let (code, out, err) = dj(&["clean"]);
    assert_eq!(code, 0, "{out}{err}");
    let (code, out, err) = dj(&["parity", "kid", "--compare", "dj-pre"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("10 passed") && out.contains("(10 frames"), "{out}");
    let (code, out, err) = dj(&["parity", "kid", "--compare", "dj-halt-pre"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("FAILED") && out.contains("not checked: in pre-roll, frame 9 of seg") && out.contains("told to halt"), "{out}");
    assert!(out.contains("synced at frame 8, 12 frames of pre-roll"), "{out}");
    let (code, out, _) = dj(&["parity", "--compare", "dj-halt-pre", "--report", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!((v["failures"][0]["segment"].as_str(), v["failures"][0]["frame"].as_u64(), v["failures"][0]["preroll"].as_u64()), (Some("seg"), Some(9), Some(12)), "{out}");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// A comparison core that crashes or hangs under `dj parity`. It shares the batch child's
/// process, so it takes the child down; the supervisor reads from the child's note that it
/// was the comparison core, and the next child records a failure at that frame, keeps the
/// frames that passed before it, and still checks the segment below.
#[test]
fn a_comparison_core_that_dies_under_dj_parity() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-dies-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (Some(core), Some(nodj)) = (build(&tmp, "testcore.so", &[]), build(&tmp, "testcore_nodj.so", &["-DNO_DJ"])) else {
        eprintln!("skipped: no C compiler");
        return;
    };
    let dir = tmp.join("project");
    project(&dir, &core, &nodj);
    std::fs::write(dir.join("segments/kid.segment"), "dj-segment 1\nid: kid\nparent: seg\nframes: 10\n---\n3 P1 JOYPAD A 1\n").unwrap();
    let dj = |args: &[&str]| -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_dj")).arg(args[0]).arg(&dir).args(&args[1..]).arg("--rom").arg(dir.join("rom.bin")).arg("--core").arg(&core).env("DJ_WORKER_TIMEOUT_SCALE", "0.2").output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (code, out, err) = dj(&["clean"]);
    assert_eq!(code, 0, "{out}{err}");

    // The test core goes wrong on the frame where its counter reaches 10: frame 9 of `seg`.
    for (name, how, call) in [("dj-crash", "crashed", "dj_handle_event"), ("dj-hang", "made no progress", "dj_handle_event")] {
        let (code, out, err) = dj(&["parity", "--compare", name]);
        assert_eq!(code, 1, "{name}: {out}{err}");
        // The supervisor says which core it was, and where.
        assert!(err.contains("the comparison core") && err.contains(how) && err.contains(call) && err.contains("at frame 9"), "{name}: {err}");
        assert!(!err.contains("left out"), "{name}: the baseline was blamed: {err}");
        // A failure at that frame, with the nine frames before it kept as passes.
        let seg = out.lines().find(|l| l.contains(" seg ")).unwrap_or_else(|| panic!("{name}: no line for seg: {out}"));
        assert!(seg.starts_with("FAILED") && seg.contains("9 passed") && seg.contains("frame 9") && seg.contains("The comparison core"), "{name}: {seg}");
        // The segment below is still checked, and passes.
        let kid = out.lines().find(|l| l.contains(" kid ")).unwrap_or_else(|| panic!("{name}: no line for kid: {out}"));
        assert!(kid.trim_start().starts_with("passed") && kid.contains("10 passed"), "{name}: {kid}");
        // The failure is kept like any other.
        let (code, out, err) = dj(&["parity", "--compare", name, "--report", "--json"]);
        assert_eq!(code, 0, "{name}: {out}{err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let fs = v["failures"].as_array().unwrap();
        assert_eq!(fs.len(), 1, "{name}: {out}");
        assert_eq!((fs[0]["segment"].as_str(), fs[0]["frame"].as_u64()), (Some("seg"), Some(9)), "{name}: {out}");
        // No note is left behind.
        let left: Vec<_> = std::fs::read_dir(dir.join(".local")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("crash-batch-")).collect();
        assert!(left.is_empty(), "{name}: {left:?}");
    }
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
