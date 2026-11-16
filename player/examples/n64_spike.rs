//! Scratch tool for .agents/yield-parity/PLAN.md, build step 2: a breakpoint in the
//! Mupen64Plus-Next connector that calls dj and waits, inside `retro_run`.
//!
//! Needs a core built from the spike copy of the connector (build/mupen64plus-spike-src),
//! which adds `dj_spike_set_breaks`, `dj_spike_suppressed` and `dj_spike_registers`. The call
//! can be made two ways: on the game's coroutine ("direct"), or by switching to the frontend's
//! coroutine and calling from `retro_run` ("via main"). Plays one segment several times and
//! reports whether hits and hashes repeat, from power-on, from a snapshot, armed and not, in
//! both ways; what a call costs; whether a write inside the call holds; the byte order of
//! memory; and the registers.
//!
//! Usage: n64_spike <project> <rom> <core> <segment> <0xOFFSET,...> [frames]
//!   offsets are into RDRAM. SPIKE_QUICK=1 skips the cost and write tests.
use anyhow::{anyhow, Result};
use decomp_dj::collection::Collection;
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::player::{PlayOptions, Session};
use std::path::Path;
use std::time::Instant;

type BreakFn = extern "C" fn(i32, i64);
type SetBreaks = unsafe extern "C" fn(*const i64, i32, Option<BreakFn>, i32);
type RegsFn = unsafe extern "C" fn(*mut u64) -> bool;
type CountFn = unsafe extern "C" fn() -> u64;

#[derive(Clone, PartialEq, Debug)]
struct Hit {
    frame: u64,
    offset: i64,
    ram: [u8; 8],
    /// PC, then r0..r31.
    regs: [u64; 33],
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Count,
    Record,
}

/// A write to make inside the call: at the `nth` hit of `offset`, zero the word at RDRAM `addr`.
#[derive(Clone)]
struct Poke {
    offset: i64,
    nth: u64,
    addr: usize,
    seen: u64,
    /// (frame, old word, word read back)
    done: Option<(u64, u32, u32)>,
}

struct Spike {
    mode: Mode,
    ram: *mut u8,
    len: usize,
    regs: Option<RegsFn>,
    frame: u64,
    count: u64,
    hits: Vec<Hit>,
    poke: Option<Poke>,
}

static mut SPIKE: Spike = Spike { mode: Mode::Count, ram: std::ptr::null_mut(), len: 0, regs: None, frame: 0, count: 0, hits: Vec::new(), poke: None };

fn sp() -> &'static mut Spike {
    unsafe { &mut *std::ptr::addr_of_mut!(SPIKE) }
}

/// What the connector calls at a breakpoint. The baseline is inside `retro_run`, about to run
/// the instruction at the breakpoint, and waits for this to return.
extern "C" fn on_break(_region: i32, offset: i64) {
    let s = sp();
    s.count += 1;
    if let Some(p) = &mut s.poke {
        if p.offset == offset && p.done.is_none() {
            p.seen += 1;
            if p.seen == p.nth {
                unsafe {
                    let at = s.ram.add(p.addr) as *mut u32;
                    let old = at.read_unaligned();
                    at.write_unaligned(0);
                    p.done = Some((s.frame, old, at.read_unaligned()));
                }
            }
        }
    }
    if s.mode == Mode::Record {
        let ram = unsafe { std::slice::from_raw_parts(s.ram, s.len) };
        let mut h = [0u8; 8];
        h.copy_from_slice(&blake3::hash(ram).as_bytes()[..8]);
        let mut regs = [0u64; 33];
        if let Some(f) = s.regs {
            unsafe { f(regs.as_mut_ptr()) };
        }
        s.hits.push(Hit { frame: s.frame, offset, ram: h, regs });
    }
}

struct Bench<'a> {
    s: Session,
    col: &'a Collection,
    id: String,
    set: SetBreaks,
    offsets: Vec<i64>,
    /// The state at frame 0 of the segment, taken the first time it is reached from power-on.
    start: Option<Vec<u8>>,
    /// The project's state hash (RAM and SRAM) after each frame of the last hashed run.
    full: Vec<String>,
}

impl Bench<'_> {
    /// Puts the baseline at frame 0 of the segment: from power-on the first time, by loading
    /// the state taken then afterwards.
    fn enter(&mut self) -> Result<()> {
        self.disarm();
        match &self.start {
            None => {
                // Not enter_replay followed by a snapshot: that saves twice with no frame
                // between, and on this core the second save runs the game on to its next
                // interrupt first, so it is a state from inside the frame.
                let chain: Vec<String> = self.col.chain(&self.id)?.into_iter().map(String::from).collect();
                let state = if chain.len() == 1 {
                    self.s.start_trunk(self.col, &self.col.replays[&self.id])?;
                    self.s.snapshot()?
                } else {
                    let res = self.s.play_chain(self.col, &chain[chain.len() - 2], &PlayOptions::default(), false)?;
                    res.last().ok_or_else(|| anyhow!("empty chain"))?.end_state.clone()
                };
                self.s.core.coverage_clear();
                self.s.restore_inputs(self.col, &self.id, 0)?;
                self.start = Some(state);
            }
            Some(st) => {
                let st = st.clone();
                self.s.load_state(&st)?;
                self.s.restore_inputs(self.col, &self.id, 0)?;
            }
        }
        Ok(())
    }

    fn arm_at(&mut self, offsets: &[i64], mode: Mode, via_main: bool, poke: Option<Poke>) {
        let s = sp();
        s.mode = mode;
        s.count = 0;
        s.hits.clear();
        s.poke = poke;
        unsafe { (self.set)(offsets.as_ptr(), offsets.len() as i32, Some(on_break), via_main as i32) };
    }

    fn arm(&mut self, mode: Mode, via_main: bool, poke: Option<Poke>) {
        let o = self.offsets.clone();
        self.arm_at(&o, mode, via_main, poke);
    }

    fn disarm(&mut self) {
        unsafe { (self.set)(std::ptr::null(), 0, None, 0) };
        let s = sp();
        s.count = 0;
        s.hits.clear();
        s.poke = None;
    }

    /// Plays frames `from..to`. With `hash`, returns the state hash after each.
    fn run(&mut self, from: u64, to: u64, hash: bool) -> Result<Vec<String>> {
        let mut out = vec![];
        for f in from..to {
            sp().frame = f;
            self.s.step(self.col, &self.id, f, false)?;
            if hash {
                // RDRAM alone is what the checks compare. The project's state hash also covers
                // SRAM, which this core's save states do not restore.
                let ram = unsafe { std::slice::from_raw_parts(sp().ram, sp().len) };
                out.push(blake3::hash(ram).to_hex()[..16].to_string());
                self.full.push(self.s.state_hash(self.col)?);
            }
        }
        Ok(out)
    }

    fn hits(&self) -> Vec<Hit> {
        sp().hits.clone()
    }
}

fn first_diff<T: PartialEq>(a: &[T], b: &[T]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b).position(|(x, y)| x != y)
}

fn same<T: PartialEq>(what: &str, a: &[T], b: &[T]) -> bool {
    match first_diff(a, b) {
        None => {
            println!("  ok    {what} ({} compared)", a.len());
            true
        }
        Some(i) => {
            println!("  FAIL  {what}: first difference at index {i} (lengths {} and {})", a.len(), b.len());
            false
        }
    }
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        return Err(anyhow!("usage: n64_spike <project> <rom> <core> <segment> <0xOFFSET,...> [frames]"));
    }
    let quick = std::env::var_os("SPIKE_QUICK").is_some();
    let mut col = Collection::open(Path::new(&a[1]))?;
    // SPIKE_NO_FB=1: keep the GPU from writing the picture into RDRAM, which dj hashes. The
    // copy is asynchronous and the GPU's buffers are not part of a save state.
    // SPIKE_FB_ASYNC=1: the GPU's copies on, whatever the project says.
    if std::env::var_os("SPIKE_FB_ASYNC").is_some() {
        col.config.core.options.insert("mupen64plus-EnableCopyColorToRDRAM".into(), "Async".into());
        col.config.core.options.insert("mupen64plus-EnableCopyDepthToRDRAM".into(), "Software".into());
        println!("GPU copies to RDRAM: on (Async colour, Software depth)");
    }
    if std::env::var_os("SPIKE_NO_FB").is_some() {
        col.config.core.options.insert("mupen64plus-EnableCopyColorToRDRAM".into(), "Off".into());
        col.config.core.options.insert("mupen64plus-EnableCopyDepthToRDRAM".into(), "Off".into());
        println!("GPU copies to RDRAM: off (not the project's setting)");
    }
    let id = col.resolve(&a[4])?.to_string();
    let full = col.replays[&id].frames()?;
    let n = a.get(6).and_then(|f| f.parse::<u64>().ok()).map_or(full, |f| f.min(full));
    let mut s = Session::open_with(&col, Path::new(&a[2]), Path::new(&a[3]), false)?;
    println!("core build: {}", s.build);
    let ptr = |s: &Session, name: &str| -> Result<*mut std::ffi::c_void> {
        let p = s.core.proc(name);
        if p.is_null() {
            return Err(anyhow!("this core has no {name}: build it from build/mupen64plus-spike-src"));
        }
        Ok(p)
    };
    // SPIKE_PLAIN=1: a core without the spike's functions. Only compares a run from power-on
    // with runs from a loaded state, to check what a save state restores.
    let plain = std::env::var_os("SPIKE_PLAIN").is_some();
    unsafe extern "C" fn no_set(_: *const i64, _: i32, _: Option<BreakFn>, _: i32) {}
    unsafe extern "C" fn no_count() -> u64 {
        0
    }
    unsafe extern "C" fn no_regs(_: *mut u64) -> bool {
        false
    }
    let set: SetBreaks = if plain { no_set } else { unsafe { std::mem::transmute(ptr(&s, "dj_spike_set_breaks")?) } };
    let suppressed: CountFn = if plain { no_count } else { unsafe { std::mem::transmute(ptr(&s, "dj_spike_suppressed")?) } };
    let regs: RegsFn = if plain { no_regs } else { unsafe { std::mem::transmute(ptr(&s, "dj_spike_registers")?) } };
    let offsets: Vec<i64> = a[5].split(',').map(|o| i64::from_str_radix(o.trim().trim_start_matches("0x"), 16)).collect::<std::result::Result<_, _>>()?;
    let names: Vec<String> = offsets.iter().map(|o| format!("RDRAM:0x{o:X}")).collect();
    {
        let ram = s.core.memory(MEMORY_SYSTEM_RAM).ok_or_else(|| anyhow!("no system RAM"))?;
        let sp = sp();
        sp.ram = ram.as_mut_ptr();
        sp.len = ram.len();
        sp.regs = Some(regs);
        println!("system RAM: {} bytes", ram.len());
    }
    let mut b = Bench { s, col: &col, id: id.clone(), set, offsets: offsets.clone(), start: None, full: vec![] };
    println!("segment {} \"{}\", {n} of {full} frames; breakpoints {}", &id[..8], col.replays[&id].title(), names.join(", "));
    let mut all_ok = true;

    // SPIKE_PAGES=1: which 4 KB pages of RDRAM differ between a run from power-on and a run
    // from a loaded state, frame by frame. With the GPU's copies on, that is where they land.
    if std::env::var_os("SPIKE_PAGES").is_some() {
        const PAGE: usize = 4096;
        let pages = |b: &mut Bench, from: u64, to: u64, mut each: Box<dyn FnMut(u64, Vec<u64>) + '_>| -> Result<()> {
            for f in from..to {
                b.s.step(b.col, &b.id, f, false)?;
                let ram = unsafe { std::slice::from_raw_parts(sp().ram, sp().len) };
                each(f, ram.chunks(PAGE).map(|c| u64::from_le_bytes(blake3::hash(c).as_bytes()[..8].try_into().unwrap())).collect());
            }
            Ok(())
        };
        let mut first: Vec<Vec<u64>> = vec![];
        b.enter()?;
        pages(&mut b, 0, n, Box::new(|_, p| first.push(p)))?;
        let np = first[0].len();
        // per page: frames differing, first and last such frame
        let mut tally = vec![(0u64, u64::MAX, 0u64); np];
        b.enter()?;
        pages(&mut b, 0, n, Box::new(|f, p| {
            for (i, (x, y)) in first[f as usize].iter().zip(&p).enumerate() {
                if x != y {
                    let t = &mut tally[i];
                    t.0 += 1;
                    t.1 = t.1.min(f);
                    t.2 = t.2.max(f);
                }
            }
        }))?;
        let differing = tally.iter().filter(|t| t.0 > 0).count();
        println!("{differing} of {np} pages differ in some frame ({} KB of {} KB)", differing * PAGE / 1024, np * PAGE / 1024);
        let mut i = 0;
        while i < np {
            if tally[i].0 == 0 {
                i += 1;
                continue;
            }
            let start = i;
            let (mut most, mut lo, mut hi) = (0, u64::MAX, 0);
            while i < np && tally[i].0 > 0 {
                most = most.max(tally[i].0);
                lo = lo.min(tally[i].1);
                hi = hi.max(tally[i].2);
                i += 1;
            }
            println!("  RDRAM 0x{:06X}-0x{:06X}  {:5} KB  differs in up to {most} frames, frames {lo} to {hi}", start * PAGE, i * PAGE - 1, (i - start) * PAGE / 1024);
        }
        return Ok(());
    }

    if plain {
        let span = |x: &[String], y: &[String]| -> String {
            let d: Vec<usize> = x.iter().zip(y).enumerate().filter(|(_, (p, q))| p != q).map(|(i, _)| i).collect();
            match (d.first(), d.last()) {
                (Some(f), Some(l)) => format!("{} frames differ, from {f} to {l}", d.len()),
                _ => "no frame differs".to_string(),
            }
        };
        b.enter()?;
        let ram_a = b.run(0, n, true)?;
        let full_a = std::mem::take(&mut b.full);
        b.enter()?;
        let ram_b = b.run(0, n, true)?;
        let full_b = std::mem::take(&mut b.full);
        let start = b.start.clone().unwrap();
        println!("save state: {} bytes", start.len());
        println!("power-on against a loaded state, after each frame: RDRAM: {}; the project's state hash (RAM and SRAM): {}", span(&ram_a, &ram_b), span(&full_a, &full_b));
        if n == full {
            let want = col.replays[&id].get("end-hash").unwrap_or("");
            println!("end hash {}, recorded {want}: {}", full_a.last().cloned().unwrap_or_default(), if full_a.last().map(String::as_str) == Some(want) { "the same" } else { "different" });
        }
        // A state from before the save storage was added is this much shorter.
        const OLD: usize = 16788288 + 1024 + 4 + 4096;
        if start.len() > OLD {
            b.s.load_state(&start[..OLD])?;
            b.s.restore_inputs(&col, &id, 0)?;
            let ram_c = b.run(0, n, true)?;
            let full_c = std::mem::take(&mut b.full);
            println!("a state cut to the old length ({OLD} bytes) loads; against power-on: RDRAM: {}; the state hash: {}", span(&ram_a, &ram_c), span(&full_a, &full_c));
        }
        return Ok(());
    }

    // ---- 1: the call on the game's coroutine, from power-on and from a snapshot --------------
    println!("\n1. The call made on the game's coroutine");
    let t = Instant::now();
    b.enter()?;
    println!("  reached the segment's start from power-on in {:.1} s", t.elapsed().as_secs_f64());
    b.arm(Mode::Record, false, None);
    let hash_a = b.run(0, n, true)?;
    let full_a = std::mem::take(&mut b.full);
    let hits_a = b.hits();
    println!("  it returned: {} calls, no crash", hits_a.len());
    for (o, name) in offsets.iter().zip(&names) {
        let k = hits_a.iter().filter(|h| h.offset == *o).count();
        let frames: std::collections::BTreeSet<u64> = hits_a.iter().filter(|h| h.offset == *o).map(|h| h.frame).collect();
        let most = frames.iter().map(|f| hits_a.iter().filter(|h| h.offset == *o && h.frame == *f).count()).max().unwrap_or(0);
        println!("  {name}: {k} hits in {} of {n} frames, at most {most} in one frame; first in frame {:?}", frames.len(), frames.iter().next());
    }
    b.enter()?;
    b.arm(Mode::Record, false, None);
    let hash_b = b.run(0, n, true)?;
    let full_b = std::mem::take(&mut b.full);
    let hits_b = b.hits();
    let span = |x: &[String], y: &[String]| -> String {
        let d: Vec<usize> = x.iter().zip(y).enumerate().filter(|(_, (p, q))| p != q).map(|(i, _)| i).collect();
        match (d.first(), d.last()) {
            (Some(f), Some(l)) => format!("{} frames differ, from {f} to {l}", d.len()),
            _ => "no frame differs".to_string(),
        }
    };
    println!("  note  after each frame, power-on against loaded: RDRAM: {}; the project's state hash (RAM and SRAM): {}", span(&hash_a, &hash_b), span(&full_a, &full_b));
    all_ok &= same("hits, from power-on and from a loaded state", &hits_a, &hits_b);
    all_ok &= same("RDRAM hash after every frame, the same two runs", &hash_a, &hash_b);

    // ---- 2: the call made from retro_run, on the frontend's coroutine -----------------------
    println!("\n2. The call made from retro_run, after a switch to the frontend's coroutine");
    b.enter()?;
    b.arm(Mode::Record, true, None);
    let hash_c = b.run(0, n, true)?;
    let hits_c = b.hits();
    all_ok &= same("hits, against the direct call", &hits_a, &hits_c);
    all_ok &= same("RDRAM hash after every frame, against the direct call", &hash_a, &hash_c);

    // ---- 3: against no breakpoints ----------------------------------------------------------
    println!("\n3. Against a run with no breakpoints");
    b.enter()?;
    let hash_d = b.run(0, n, true)?;
    all_ok &= same("RDRAM hash after every frame, armed and not", &hash_a, &hash_d);
    all_ok &= same("the same, both from a loaded state", &hash_b, &hash_d);
    if n == full {
        match col.replays[&id].get("end-hash") {
            Some(want) if full_a.last().map(String::as_str) == Some(want) => println!("  ok    the segment's recorded end-hash is reached"),
            Some(want) => println!("  NOTE  end hash {} but the segment records {want}", full_a.last().cloned().unwrap_or_default()),
            None => println!("  (the segment records no end-hash)"),
        }
    }

    // ---- 4: a snapshot mid-segment, both ways -----------------------------------------------
    let k = n / 2;
    for via_main in [false, true] {
        println!("\n4{}. A snapshot at frame {k}, the call {}", if via_main { "b" } else { "a" }, if via_main { "via retro_run" } else { "direct" });
        b.enter()?;
        b.arm(Mode::Record, via_main, None);
        b.run(0, k, false)?;
        let before = unsafe { suppressed() };
        sp().frame = u64::MAX; // a hit delivered while saving or loading would carry this
        let snap = b.s.snapshot()?;
        let after_save = unsafe { suppressed() };
        sp().hits.clear();
        let tail1 = b.run(k, n, true)?;
        let hits1 = b.hits();
        sp().frame = u64::MAX;
        let before_load = unsafe { suppressed() };
        b.s.load_state(&snap)?;
        b.s.restore_inputs(&col, &id, k)?;
        let after_load = unsafe { suppressed() };
        let during = sp().hits.iter().filter(|h| h.frame == u64::MAX).count();
        sp().hits.clear();
        let tail2 = b.run(k, n, true)?;
        let hits2 = b.hits();
        all_ok &= same("hits after the snapshot was loaded", &hits1, &hits2);
        all_ok &= same("RDRAM hashes after the snapshot was loaded", &tail1, &tail2);
        all_ok &= same("RDRAM hashes against the straight run", &tail1, &hash_a[k as usize..]);
        let tail_a: Vec<Hit> = hits_a.iter().filter(|h| h.frame >= k).cloned().collect();
        all_ok &= same("hits against the straight run", &hits1, &tail_a);
        println!("  {}    calls delivered while saving or loading: {during}", if during == 0 { "ok" } else { "FAIL" });
        all_ok &= during == 0;
        println!("  note  hits held back by the connector: {} while saving, {} while loading", after_save - before, after_load - before_load);
    }

    // ---- 5: byte order and registers --------------------------------------------------------
    println!("\n5. Memory and registers");
    {
        let ram = unsafe { std::slice::from_raw_parts(sp().ram, sp().len) };
        let at = 0x570;
        let native = u32::from_ne_bytes(ram[at..at + 4].try_into().unwrap());
        println!("  the word at RDRAM 0x570 is `jal rng_main`, 0x0C00D844 on the console");
        println!("  bytes in the handle's memory: {:02X} {:02X} {:02X} {:02X}; read as a host word: 0x{native:08X}", ram[at], ram[at + 1], ram[at + 2], ram[at + 3]);
        println!("  {}", if native == 0x0C00D844 { "so memory is 32-bit words in the host's order: a byte at address A is at A ^ 3 on this machine" } else if ram[at..at + 4] == [0x0C, 0x00, 0xD8, 0x44] { "so memory is in the console's byte order" } else { "which is neither order: the word is not there at this point" });
    }
    let wrong = hits_a.iter().filter(|h| h.regs[0] as i64 != 0x8000_0000 + h.offset).count();
    println!("  {}    PC is the breakpoint's address at {} of {} hits", if wrong == 0 { "ok" } else { "FAIL" }, hits_a.len() - wrong, hits_a.len());
    all_ok &= wrong == 0;
    for (o, name) in offsets.iter().zip(&names) {
        let sps: std::collections::BTreeSet<u64> = hits_a.iter().filter(|h| h.offset == *o).map(|h| h.regs[1 + 29] & 0xFFFF_FFFF).collect();
        println!("  {name}: stack pointer values seen: {}", sps.iter().take(4).map(|v| format!("0x{v:08X}")).collect::<Vec<_>>().join(", "));
    }
    for h in hits_a.iter().take(2) {
        println!("  frame {:4} RDRAM:0x{:X}  PC={:08X} sp={:08X} ra={:08X} a0={:08X}", h.frame, h.offset, h.regs[0], h.regs[30] & 0xFFFF_FFFF, h.regs[32] & 0xFFFF_FFFF, h.regs[5] & 0xFFFF_FFFF);
    }

    if !quick {
        // ---- 6: cost ------------------------------------------------------------------------
        println!("\n6. Cost ({n} frames, best of 2)");
        let time = |b: &mut Bench, at: Option<(&[i64], bool)>| -> Result<(f64, u64)> {
            let mut best = f64::MAX;
            let mut count = 0;
            for _ in 0..2 {
                b.enter()?;
                if let Some((o, via)) = at {
                    b.arm_at(o, Mode::Count, via, None);
                }
                let t = Instant::now();
                b.run(0, n, false)?;
                best = best.min(t.elapsed().as_secs_f64());
                count = sp().count;
            }
            Ok((best, count))
        };
        let (t_off, _) = time(&mut b, None)?;
        println!("  not armed:                         {:7.1} frames/s", n as f64 / t_off);
        let (t_idle, _) = time(&mut b, Some((&[0x7F_FFFC], false)))?;
        println!("  armed where nothing runs:          {:7.1} frames/s", n as f64 / t_idle);
        let (t_dir, c_dir) = time(&mut b, Some((&offsets, false)))?;
        println!("  armed, direct:                     {:7.1} frames/s  ({c_dir} calls)", n as f64 / t_dir);
        let (t_via, c_via) = time(&mut b, Some((&offsets, true)))?;
        println!("  armed, via retro_run:              {:7.1} frames/s  ({c_via} calls)", n as f64 / t_via);
        // The idle thread's call of rng_main runs whenever the CPU has nothing to do.
        let hot: &[i64] = &[0x570];
        let (t_hd, c_hd) = time(&mut b, Some((hot, false)))?;
        println!("  armed in the idle loop, direct:    {:7.1} frames/s  ({c_hd} calls, {:.0} per frame): {:.0} ns a call", n as f64 / t_hd, c_hd as f64 / n as f64, (t_hd - t_idle) / c_hd.max(1) as f64 * 1e9);
        let (t_hv, c_hv) = time(&mut b, Some((hot, true)))?;
        println!("  armed in the idle loop, via main:  {:7.1} frames/s  ({c_hv} calls, {:.0} per frame): {:.0} ns a call", n as f64 / t_hv, c_hv as f64 / n as f64, (t_hv - t_idle) / c_hv.max(1) as f64 * 1e9);

        // ---- 7: a write inside the call -----------------------------------------------------
        let tick = offsets[0];
        let ticks = hits_a.iter().filter(|h| h.offset == tick).count() as u64;
        println!("\n7. A write inside the call: zero the word at RDRAM 0x570 (`jal rng_main` becomes a nop) at hit {} of {}", ticks / 2, names[0]);
        if ticks < 4 {
            println!("  (too few hits to try)");
        } else {
            let poke = Poke { offset: tick, nth: ticks / 2, addr: 0x570, seen: 0, done: None };
            b.enter()?;
            b.arm(Mode::Record, false, Some(poke.clone()));
            let w1 = b.run(0, n, true)?;
            let done = sp().poke.as_ref().and_then(|p| p.done);
            b.enter()?;
            b.arm(Mode::Record, false, Some(poke.clone()));
            let w2 = b.run(0, n, true)?;
            match done {
                None => {
                    println!("  FAIL  the write was never made");
                    all_ok = false;
                }
                Some((f, old, back)) => {
                    println!("  written in frame {f}: 0x{old:08X} -> 0, read back 0x{back:08X}");
                    all_ok &= same("RDRAM hashes of two runs with the write", &w1, &w2);
                    match first_diff(&w1, &hash_a) {
                        Some(i) => println!("  ok    the write takes effect: hashes differ from the plain run from frame {i} on (written in frame {f})"),
                        None => {
                            println!("  FAIL  the write changed nothing");
                            all_ok = false;
                        }
                    }
                    let k2 = (f + 10).min(n - 1);
                    b.enter()?;
                    b.arm(Mode::Record, false, Some(poke.clone()));
                    b.run(0, k2, false)?;
                    let snap = b.s.snapshot()?;
                    let t1 = b.run(k2, n, true)?;
                    b.s.load_state(&snap)?;
                    b.s.restore_inputs(&col, &id, k2)?;
                    let t2 = b.run(k2, n, true)?;
                    all_ok &= same(&format!("RDRAM hashes after a snapshot at frame {k2}, past the write, was loaded"), &t1, &t2);
                    all_ok &= same("those against the straight run with the write", &t1, &w1[k2 as usize..]);
                }
            }
        }
    }

    println!("\n{}", if all_ok { "All checks passed." } else { "Some checks FAILED." });
    b.disarm();
    Ok(())
}
