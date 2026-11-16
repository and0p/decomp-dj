//! A comparison core that takes events (`dj_handle_event`, `dj_handle_write`,
//! `dj_breakpoints`), writes at an instruction, and dj's own tick count, end to end with the
//! test core (.agents/yield-parity/PLAN.md, build steps 5 and 6; the cases are those of
//! .agents/yield-parity/STATES.md).
//!
//! The baseline is the test core built with a small dj connector (-DCONNECTOR): a frame
//! "runs" a fixed pattern of instructions, with a tick that ends between the frame's two
//! halves and no tick on every seventh frame. The comparison core is the same core built to
//! take events (-DEVENTS). Skipped when there is no C compiler.

use decomp_dj::collection::Collection;
use decomp_dj::compare::Comparison;
use decomp_dj::coverage::FLAG_OPCODE;
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::parity::{check_span, Checked, Checker, Span};
use decomp_dj::player::{PlayOptions, Session};
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::process::Command;

fn build(out: &Path, name: &str, defs: &[&str]) -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testcore/testcore.c");
    let lib = out.join(name);
    let ok = Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&lib).arg(&src).args(defs).status().ok()?.success();
    ok.then_some(lib)
}

/// 40 frames. B is held in frame 0 alone, and Y in frames 6 to 8, so the instructions at
/// 0x21 and 0x22 run only there.
const SEG: &str = "dj-segment 1\nid: seg\nparent: none\nframes: 40\n---\n0 P1 JOYPAD B 1\n1 P1 JOYPAD B 0\n6 P1 JOYPAD Y 1\n9 P1 JOYPAD Y 0\n";

struct Cores {
    tmp: PathBuf,
    base: PathBuf,
    events: PathBuf,
    no_write: PathBuf,
}

fn cores(what: &str) -> Option<Cores> {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("events-{what}-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let base = build(&tmp, "base.so", &["-DCONNECTOR", "-DNO_DJ"])?;
    let events = build(&tmp, "events.so", &["-DEVENTS"])?;
    let no_write = build(&tmp, "no_write.so", &["-DEVENTS", "-DNO_WRITE_HANDLER"])?;
    Some(Cores { tmp, base, events, no_write })
}

/// A project whose baseline is the connector core. `extra` goes into config.toml; `segments`
/// are (id, body lines) added beside `seg`.
fn project(c: &Cores, extra: &str, segments: &[(&str, &str)]) -> Collection {
    let dir = c.tmp.join("project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("segments")).unwrap();
    std::fs::create_dir_all(dir.join(".local")).unwrap();
    std::fs::write(dir.join("rom.bin"), b"rom").unwrap();
    std::fs::write(dir.join("segments/seg.segment"), SEG).unwrap();
    for (id, body) in segments {
        std::fs::write(dir.join(format!("segments/{id}.segment")), format!("dj-segment 1\nid: {id}\nparent: none\nframes: 40\n---\n{body}")).unwrap();
    }
    let p = |p: &Path| p.display().to_string().replace('\\', "/");
    let (base, ev, nw) = (p(&c.base), p(&c.events), p(&c.no_write));
    let entry = |name: &str, path: &str, options: &str| format!("[[compare]]\nname = \"{name}\"\npath = \"{path}\"\n{}", if options.is_empty() { String::new() } else { format!("[compare.options]\n{options}\n") });
    let config = format!(
        "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\npath = \"rom.bin\"\n[core]\nname = \"testcore\"\nbuild = \"testcore 1\"\npath = \"{base}\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n{extra}\n{}{}{}{}{}{}{}{}{}",
        entry("frame", &ev, ""),
        entry("tick", &ev, "testcore_judge = \"tick\""),
        entry("copy", &ev, "testcore_judge = \"copy\""),
        entry("mismatch", &ev, "testcore_break = \"mismatch\""),
        entry("heal", &ev, "testcore_break = \"heal\""),
        entry("halt", &ev, "testcore_break = \"halt\""),
        entry("unjudged", &ev, "testcore_break = \"unjudged\""),
        entry("no-write", &nw, ""),
        entry("tick-no-write", &nw, "testcore_judge = \"tick\""),
    );
    std::fs::write(dir.join("config.toml"), config).unwrap();
    Collection::open(&dir).unwrap()
}

fn session(c: &Cores, col: &Collection) -> Session {
    Session::open_with(col, &col.root.join("rom.bin"), &c.base, false).unwrap()
}

fn open(c: &Cores, col: &Collection, name: &str) -> Comparison {
    Comparison::open(col, name, &c.base, Some(&col.root.join("rom.bin"))).unwrap()
}

/// What the comparison core was sent: "J1 " a jump with reason 1, "R " a console reset,
/// "B1 " a break with id 1, "F " a frame end, "W " a write, "Wa " a write at an instruction.
fn sent(c: &Comparison) -> String {
    let f = c.core.proc("testcore_events");
    assert!(!f.is_null());
    let f: unsafe extern "C" fn() -> *const std::ffi::c_char = unsafe { std::mem::transmute(f) };
    unsafe { CStr::from_ptr(f()) }.to_string_lossy().into_owned()
}

/// Checks all of segment `id` against comparison core `name`: the span, the frames left
/// unjudged at the end, and what the core was sent.
fn check(c: &Cores, col: &Collection, s: &mut Session, name: &str, id: &str) -> (Span, u64, String) {
    s.enter_replay(col, id).unwrap();
    let mut cmp = open(c, col, name);
    let mut ck = Checker::default();
    let n = col.replays[id].frames().unwrap();
    let span = check_span(s, &mut cmp, &mut ck, col, id, 0, n, 0).unwrap();
    let left = ck.finish();
    let log = sent(&cmp);
    (span, left, log)
}

fn ran(flags: &[u8], off: usize) -> bool {
    flags.get(off).is_some_and(|f| f & FLAG_OPCODE != 0)
}

fn count(log: &str, what: &str) -> usize {
    log.split_whitespace().filter(|w| *w == what).count()
}

/// One at a time: the cores load into process-wide slots.
static SLOTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A core that judges at frame ends: it copies the baseline on the jump itself, so the
/// first frame after it is checked, as before events.
#[test]
fn a_core_that_judges_at_frame_ends() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("frame") else { return eprintln!("skipped: no C compiler") };
    let col = project(&c, "", &[]);
    let mut s = session(&c, &col);

    let (span, left, log) = check(&c, &col, &mut s, "frame", "seg");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, span.skipped, span.unchecked, left), (40, 0, 0, 0));
    // A fresh power-on is said so; then a frame end for each frame, and nothing else.
    assert!(log.starts_with("J1 F "), "{log}");
    assert_eq!((count(&log, "F"), log.split_whitespace().count()), (40, 41), "{log}");
    // Frame 0 is credited: the code only it ran has parity.
    assert!(ran(&span.passed_flags, 0x21) && ran(&span.passed_flags, 0x22) && ran(&span.passed_flags, 0x10), "the frames' code is credited");

    // A core that copies the baseline at every event and answers match: all pass.
    let (span, left, _) = check(&c, &col, &mut s, "copy", "seg");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, span.skipped, span.unchecked, left), (40, 0, 0, 0));
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// A core that judges at a breakpoint inside the frame, where the baseline's tick ends. It
/// can only copy the baseline there, so what ran between the jump and the copy is credited to
/// nothing; a frame with no tick is unjudged; and the frames after its last verdict are the
/// segment's tail.
#[test]
fn a_core_that_judges_at_a_tick() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("tick") else { return eprintln!("skipped: no C compiler") };
    let col = project(&c, "", &[]);
    let mut s = session(&c, &col);

    let (span, left, log) = check(&c, &col, &mut s, "tick", "seg");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    // It was sent its breakpoints: the tick's end 35 times (not on the five lag frames), and
    // the frame's start every frame.
    assert_eq!((count(&log, "B1"), count(&log, "B2"), count(&log, "F"), count(&log, "J1")), (35, 40, 40, 1), "{log}");
    // Every frame is accounted for once: passed, unjudged, or the tail.
    let tail = 40 - (span.passed + span.skipped + span.unchecked);
    assert_eq!((span.unchecked, left), (0, 0), "{span:?}");
    // Coverage is credited by whole frames, one verdict late, so a lag frame costs four:
    // the frame before it and the lag frame (still held when the tick after it is unjudged),
    // that frame, and the one the core copies the baseline in. Seven frames to a lag frame:
    // three credited, four not.
    assert_eq!((span.passed, span.skipped), (18, 21), "{span:?}");
    assert_eq!(tail, 1, "the frame after the last tick's verdict is the tail: {span:?}");
    // Frame 0 ran before the core had copied the baseline: its code is credited to nothing.
    assert!(!ran(&span.passed_flags, 0x21), "frame 0 was never compared");
    // The frames it did judge are credited, the tick's own instruction among them.
    assert!(ran(&span.passed_flags, 0x80) && ran(&span.passed_flags, 0x10) && ran(&span.passed_flags, 0x20));
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// Runs frame `k` of `seg` on both cores, as a player does.
fn frame(s: &mut Session, cmp: &mut Comparison, ck: &mut Checker, col: &Collection, k: u64) -> Checked {
    if !ck.in_step("seg", k) {
        ck.sync(cmp, &mut s.core, col, "seg", k, k).unwrap();
    }
    ck.begin(cmp, &mut s.core, col, "seg", k, false, true);
    s.step(col, "seg", k, false).unwrap();
    ck.step(cmp, &mut s.core, col, "seg", k, k, false, true).expect("in step").checked
}

/// A mismatch may put itself right, so a run of them is one failure: the first stops
/// playback and is kept, the rest are shown and playback goes on. Sync now, or the core
/// itself, starts again from the baseline.
#[test]
fn a_run_of_mismatches_is_one_failure() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("mismatch") else { return eprintln!("skipped: no C compiler") };
    let col = project(&c, "", &[]);
    let mut s = session(&c, &col);

    // The core's RAM goes wrong at frame 9, for good.
    s.enter_replay(&col, "seg").unwrap();
    let mut cmp = open(&c, &col, "mismatch");
    let mut ck = Checker::default();
    for k in 0..9 {
        assert_eq!(frame(&mut s, &mut cmp, &mut ck, &col, k), Checked::Passed, "frame {k}");
    }
    match frame(&mut s, &mut cmp, &mut ck, &col, 9) {
        Checked::Failed(msg) => assert!(msg.contains("Parity failure at frame 9, at the frame end") && msg.contains("RAM 0x02"), "{msg}"),
        other => panic!("frame 9: {other:?}"),
    }
    // Playing on: no jump, the core carries on, and the mismatches that follow are not failures.
    for k in 10..13 {
        match frame(&mut s, &mut cmp, &mut ck, &col, k) {
            Checked::Drifting(msg) => assert!(msg.contains(&format!("frame {k}")), "{msg}"),
            other => panic!("frame {k}: {other:?}"),
        }
    }
    assert_eq!(count(&sent(&cmp), "J1") + count(&sent(&cmp), "J2") + count(&sent(&cmp), "J3"), 1, "no jump after the mismatch: {}", sent(&cmp));
    // Sync now: the core copies the baseline again, and has parity from there.
    ck.copy_again();
    assert_eq!(frame(&mut s, &mut cmp, &mut ck, &col, 13), Checked::Passed);
    assert_eq!(count(&sent(&cmp), "J3"), 1, "{}", sent(&cmp));
    assert_eq!(frame(&mut s, &mut cmp, &mut ck, &col, 14), Checked::Passed);
    drop(cmp);

    // A core that puts itself right: one failure, a run that goes on, then parity again
    // without dj doing anything.
    s.enter_replay(&col, "seg").unwrap();
    let mut cmp = open(&c, &col, "heal");
    let mut ck = Checker::default();
    let got: Vec<Checked> = (0..16).map(|k| frame(&mut s, &mut cmp, &mut ck, &col, k)).collect();
    assert!(got[..9].iter().all(|g| *g == Checked::Passed), "{got:?}");
    assert!(matches!(got[9], Checked::Failed(_)), "{got:?}");
    assert!(got[10..13].iter().all(|g| matches!(g, Checked::Drifting(_))), "{got:?}");
    // It starts again at frame 13 (unjudged), copies at the end of 14, and matches from 15.
    assert!(matches!(got[13], Checked::Skipped(_)) && matches!(got[14], Checked::Skipped(_)), "{got:?}");
    assert_eq!(got[15], Checked::Passed, "{got:?}");
    drop(cmp);

    // check_span stops at the failure, with what was held before it as passes.
    let (span, _, _) = check(&c, &col, &mut s, "mismatch", "seg");
    let f = span.failure.expect("a failure");
    assert_eq!((f.frame, span.passed, f.synced.clone()), (9, 9, ("seg".to_string(), 0)));
    assert_eq!(f.diffs.total, 1, "{:?}", f.diffs);
    assert!(ran(&f.flags, 0x10), "the failing frame's code is blamed");
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// A halt stops playback and the events; playing on has the core copy the baseline again.
#[test]
fn a_halt_is_resumed_with_a_jump() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("halt") else { return eprintln!("skipped: no C compiler") };
    let col = project(&c, "", &[]);
    let mut s = session(&c, &col);
    s.enter_replay(&col, "seg").unwrap();
    let mut cmp = open(&c, &col, "halt");
    let mut ck = Checker::default();
    for k in 0..9 {
        assert_eq!(frame(&mut s, &mut cmp, &mut ck, &col, k), Checked::Passed, "frame {k}");
    }
    match frame(&mut s, &mut cmp, &mut ck, &col, 9) {
        Checked::Failed(msg) => assert!(msg.contains("The comparison core stopped at frame 9") && msg.contains("told to halt"), "{msg}"),
        other => panic!("frame 9: {other:?}"),
    }
    assert_eq!(count(&sent(&cmp), "J3"), 0);
    // Playing on: a jump, "copy again", before the frame; then parity.
    assert_eq!(frame(&mut s, &mut cmp, &mut ck, &col, 10), Checked::Passed);
    let log = sent(&cmp);
    assert_eq!(count(&log, "J3"), 1, "{log}");
    // Ten frame ends before the halt, none while halted, one after the jump.
    assert!(log.ends_with("F J3 F "), "{log}");
    drop(cmp);
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// "Unjudged" credits nothing: the code that ran only in a stretch the core couldn't judge
/// stays without parity.
#[test]
fn an_unjudged_stretch_credits_no_coverage() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("unjudged") else { return eprintln!("skipped: no C compiler") };
    let col = project(&c, "", &[]);
    let mut s = session(&c, &col);
    // Unjudged for frames 6 to 8, where Y is held; then it copies at the end of frame 9.
    let (span, left, _) = check(&c, &col, &mut s, "unjudged", "seg");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, span.skipped, span.unchecked, left), (36, 4, 0, 0), "{span:?}");
    assert!(!ran(&span.passed_flags, 0x22), "the code only the unjudged frames ran has no parity");
    assert!(ran(&span.passed_flags, 0x21) && ran(&span.passed_flags, 0x20));
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// The test core's frame, to work out what a write should leave: (counter, buttons, mix).
fn model(frames: u64, write: Option<(u64, bool)>) -> [u8; 3] {
    let mut r = [0u8; 3];
    for k in 0..frames {
        // Before the frame.
        if write == Some((k, false)) {
            r[1] = 0x77;
        }
        r[0] = r[0].wrapping_add(1);
        r[1] = 0;
        // Where the tick ends, except on a lag frame.
        if write == Some((k, true)) && r[0] % 7 != 0 {
            r[1] = 0x77;
        }
        r[2] = r[2].wrapping_mul(31).wrapping_add(r[1]).wrapping_add(1);
    }
    r
}

/// `[WRITE]` lines, between frames and at an instruction: what the baseline does with them,
/// and what the comparison core is told.
#[test]
fn writes() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("writes") else { return eprintln!("skipped: no C compiler") };
    let col = project(
        &c,
        "",
        &[
            ("plain", ""),
            // Between frames: the buttons byte, which the frame then overwrites.
            ("between", "12 [WRITE] RAM 0x01 77\n"),
            // At the instruction where the tick ends, after the frame has set that byte.
            ("at", "10 [WRITE] RAM 0x01 77 @PRG:0x80\n"),
            // Frame 6 is a lag frame: that instruction doesn't run.
            ("lag", "6 [WRITE] RAM 0x01 77 @PRG:0x80\n"),
            // A byte the frame leaves alone, to follow.
            ("spare", "12 [WRITE] RAM 0x20 ab cd\n"),
        ],
    );
    let mut s = session(&c, &col);
    let end = |s: &mut Session, id: &str| -> Vec<u8> {
        let start = s.enter_replay(&col, id).unwrap();
        s.play(&col, id, start, &PlayOptions::default()).unwrap();
        s.core.memory(MEMORY_SYSTEM_RAM).unwrap().to_vec()
    };
    let plain = end(&mut s, "plain");
    assert_eq!(plain[..3], model(40, None));
    // Between frames the write is undone by the frame; at the instruction it lands after
    // the frame set the byte, and changes what follows.
    assert_eq!(end(&mut s, "between")[..3], model(40, Some((12, false))));
    assert_eq!(end(&mut s, "between")[..3], plain[..3]);
    let at = end(&mut s, "at");
    assert_eq!(at[..3], model(40, Some((10, true))));
    assert_ne!(at[..3], plain[..3], "a write at an instruction is made inside the frame");
    // The same again: it repeats.
    assert_eq!(end(&mut s, "at"), at);
    // Its instruction didn't run in that frame: the write is dropped, and nothing waits.
    assert_eq!(end(&mut s, "lag"), plain);

    // A core parked at a frame boundary follows a write between frames.
    let (span, _, log) = check(&c, &col, &mut s, "frame", "spare");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, count(&log, "W")), (40, 1), "{log}");
    // One with no dj_handle_write can't: it is asked to copy the baseline again, at once.
    let (span, _, log) = check(&c, &col, &mut s, "no-write", "spare");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, count(&log, "W"), count(&log, "J3")), (40, 0, 1), "{log}");

    // A write at an instruction: the core is told inside the frame. One parked at a frame
    // boundary isn't there, so it starts again from the baseline: unjudged, no failure.
    let (span, _, log) = check(&c, &col, &mut s, "frame", "at");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!(count(&log, "Wa"), 1, "{log}");
    assert!(span.skipped >= 1 && span.passed >= 38, "{span:?}");
    // A core with a breakpoint at that instruction applies the write and stays in parity:
    // the frame is judged, not skipped. The write comes before the break.
    let (base, _, _) = check(&c, &col, &mut s, "tick", "plain");
    let (span, _, log) = check(&c, &col, &mut s, "tick", "at");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert!(log.contains("Wa B1 "), "{log}");
    assert_eq!((span.passed, span.skipped), (base.passed, base.skipped), "the write costs no frame: {span:?} against {base:?}");
    // The same core without dj_handle_write: the rest of that frame is unjudged, and it
    // copies the baseline again.
    let (span, _, log) = check(&c, &col, &mut s, "tick-no-write", "at");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((count(&log, "Wa"), count(&log, "J3")), (0, 1), "{log}");
    assert!(span.skipped > base.skipped, "{span:?} against {base:?}");
    // The dropped write: the core is told nothing.
    let (span, _, log) = check(&c, &col, &mut s, "tick", "lag");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((count(&log, "Wa"), span.passed, span.skipped), (0, base.passed, base.skipped), "{log}");
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// A `[RESET]` line is the console's reset: the baseline gets it, and the core is told, to
/// run its own reset path.
#[test]
fn a_console_reset_is_an_event() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("reset") else { return eprintln!("skipped: no C compiler") };
    let col = project(&c, "", &[("reset", "15 [RESET]\n")]);
    let mut s = session(&c, &col);
    // Parked at a frame boundary: it resets itself and stays in parity.
    let (span, _, log) = check(&c, &col, &mut s, "frame", "reset");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!((span.passed, count(&log, "R")), (40, 1), "{log}");
    // Fifteen frame ends, the reset, then the rest.
    assert!(log.contains("F R F "), "{log}");
    // Parked at a tick: it can't reset from the right state, so it starts again.
    let (base, _, _) = check(&c, &col, &mut s, "tick", "seg");
    let (span, _, log) = check(&c, &col, &mut s, "tick", "reset");
    assert!(span.failure.is_none(), "{:?}", span.failure);
    assert_eq!(count(&log, "R"), 1, "{log}");
    assert!(span.passed > 10 && span.passed <= base.passed, "{span:?} against {base:?}");
    drop(s);
    let _ = std::fs::remove_dir_all(&c.tmp);
}

/// dj counts ticks itself at a breakpoint; the cap switches one in a loop off for the frame.
#[test]
fn ticks_are_counted_by_dj() {
    let _slots = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = cores("ticks") else { return eprintln!("skipped: no C compiler") };
    let ticks = |extra: &str| -> Option<u64> {
        let col = project(&c, extra, &[]);
        let mut s = session(&c, &col);
        let start = s.enter_replay(&col, "seg").unwrap();
        // The coverage flags are cleared after every frame, as a parity check does.
        let mut rec = s.begin(&col, "seg", start, &PlayOptions::default()).unwrap();
        loop {
            let done = s.advance(&col, &mut rec, false).unwrap();
            s.core.coverage_clear();
            if done {
                break;
            }
        }
        s.finish(&col, rec).unwrap().ticks
    };
    // A tick on every frame but the five lag frames.
    assert_eq!(ticks("[tick]\nexec = \"PRG:0x80\"\n"), Some(35));
    // A wait that spins 1,500 times on frames 4, 20 and 36: counted up to the cap each time.
    assert_eq!(ticks("[tick]\nexec = \"PRG:0x90\"\n"), Some(3 * decomp_dj::libretro::BREAK_CAP as u64));
    assert_eq!(ticks(""), None);
    let _ = std::fs::remove_dir_all(&c.tmp);
}
