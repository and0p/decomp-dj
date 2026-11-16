//! Scratch tool for .agents/yield-parity/PLAN.md, build step 1: a breakpoint in the Mesen
//! connector that calls dj and waits, inside `retro_run`.
//!
//! Needs a core built from the spike copy of the connector (build/mesen-spike-src), which adds
//! `dj_spike_set_breaks` and `dj_spike_registers`. Plays one segment of a project several times
//! and reports:
//!   1. whether the hits (frame, RAM hash, registers) are the same across two runs;
//!   2. whether the frame hashes are the same with and without the breakpoints;
//!   3. whether they are the same after loading a snapshot taken mid-segment;
//!   4. what a hit costs;
//!   5. whether writing RAM inside the call takes effect and stays repeatable;
//!   6. what the registers read at a hit.
//!
//! Usage: mesen_spike <project> <rom> <core> <segment> <REGION:0xOFFSET,...> <RAM address to write>
use anyhow::{anyhow, Result};
use decomp_dj::collection::Collection;
use decomp_dj::libretro::{MEMORY_SYSTEM_RAM, TRACE_EXEC};
use decomp_dj::player::{parse_region_offset, Session};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

type BreakFn = extern "C" fn(i32, i64);
type SetBreaks = unsafe extern "C" fn(*const i32, *const i64, i32, Option<BreakFn>);
type RegsFn = unsafe extern "C" fn(*mut u8) -> bool;

#[derive(Clone, PartialEq, Debug)]
struct Hit {
    frame: u64,
    offset: i64,
    ram: [u8; 8],
    regs: [u8; 7],
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Count,
    Record,
}

/// A write to make inside the call: at the `nth` hit of `offset`, add one to RAM[addr].
#[derive(Clone)]
struct Poke {
    offset: i64,
    nth: u64,
    addr: usize,
    seen: u64,
    /// (frame, old value, new value, value read back)
    done: Option<(u64, u8, u8, u8)>,
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

/// What the connector calls at a breakpoint. The baseline is inside `retro_run`, in the opcode
/// fetch of the instruction at the breakpoint, and waits for this to return.
extern "C" fn on_break(_region: i32, offset: i64) {
    let s = sp();
    s.count += 1;
    if let Some(p) = &mut s.poke {
        if p.offset == offset && p.done.is_none() {
            p.seen += 1;
            if p.seen == p.nth {
                unsafe {
                    let at = s.ram.add(p.addr);
                    let old = *at;
                    let new = old.wrapping_add(1);
                    *at = new;
                    p.done = Some((s.frame, old, new, *at));
                }
            }
        }
    }
    if s.mode == Mode::Record {
        let ram = unsafe { std::slice::from_raw_parts(s.ram, s.len) };
        let mut h = [0u8; 8];
        h.copy_from_slice(&blake3::hash(ram).as_bytes()[..8]);
        let mut regs = [0u8; 7];
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
    regions: Vec<i32>,
    offsets: Vec<i64>,
}

impl Bench<'_> {
    /// Puts the baseline at frame 0 of the segment, from power-on (the session has no database).
    fn enter(&mut self) -> Result<()> {
        self.disarm();
        self.s.enter_replay(self.col, &self.id)?;
        Ok(())
    }

    fn arm_at(&mut self, regions: &[i32], offsets: &[i64], mode: Mode, poke: Option<Poke>) {
        let s = sp();
        s.mode = mode;
        s.count = 0;
        s.hits.clear();
        s.poke = poke;
        unsafe { (self.set)(regions.as_ptr(), offsets.as_ptr(), regions.len() as i32, Some(on_break)) };
    }

    fn arm(&mut self, mode: Mode, poke: Option<Poke>) {
        let (r, o) = (self.regions.clone(), self.offsets.clone());
        self.arm_at(&r, &o, mode, poke);
    }

    fn disarm(&mut self) {
        unsafe { (self.set)(std::ptr::null(), std::ptr::null(), 0, None) };
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
                out.push(self.s.state_hash(self.col)?);
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
    if a.len() < 7 {
        return Err(anyhow!("usage: mesen_spike <project> <rom> <core> <segment> <REGION:0xOFFSET,...> <RAM address>"));
    }
    let col = Collection::open(Path::new(&a[1]))?;
    let id = col.resolve(&a[4])?.to_string();
    let n = col.replays[&id].frames()?;
    let mut s = Session::open_with(&col, Path::new(&a[2]), Path::new(&a[3]), false)?;
    println!("core build: {}", s.build);
    let set_ptr = s.core.proc("dj_spike_set_breaks");
    let regs_ptr = s.core.proc("dj_spike_registers");
    if set_ptr.is_null() || regs_ptr.is_null() {
        return Err(anyhow!("this core has no dj_spike_set_breaks: build it from build/mesen-spike-src"));
    }
    let set: SetBreaks = unsafe { std::mem::transmute(set_ptr) };
    let (mut regions, mut offsets, mut names) = (vec![], vec![], vec![]);
    for b in a[5].split(',') {
        let (r, o) = parse_region_offset(b)?;
        let i = s.core.coverage_region_index(&r).ok_or_else(|| anyhow!("the connector has no region {r}"))?;
        regions.push(i as i32);
        offsets.push(o as i64);
        names.push(format!("{r}:0x{o:X}"));
    }
    let poke_addr = usize::from_str_radix(a[6].trim_start_matches("0x"), 16)?;
    {
        let ram = s.core.memory(MEMORY_SYSTEM_RAM).ok_or_else(|| anyhow!("no system RAM"))?;
        let sp = sp();
        sp.ram = ram.as_mut_ptr();
        sp.len = ram.len();
        sp.regs = Some(unsafe { std::mem::transmute::<*mut std::ffi::c_void, RegsFn>(regs_ptr) });
    }
    let prg = s.core.coverage_region_index("PRG").map(|i| i as i8);
    let mut b = Bench { s, col: &col, id: id.clone(), set, regions, offsets, };
    println!("segment {} \"{}\", {n} frames; breakpoints {}", &id[..8], col.replays[&id].title(), names.join(", "));
    let mut all_ok = true;

    // ---- 1 and 2: the same twice, and the same as without breakpoints -----------------------
    println!("\n1. Two runs with the breakpoints armed");
    b.enter()?;
    b.arm(Mode::Record, None);
    let hash_a = b.run(0, n, true)?;
    let hits_a = b.hits();
    b.enter()?;
    b.arm(Mode::Record, None);
    let hash_b = b.run(0, n, true)?;
    let hits_b = b.hits();
    for (o, name) in b.offsets.clone().iter().zip(&names) {
        let k = hits_a.iter().filter(|h| h.offset == *o).count();
        let frames: std::collections::BTreeSet<u64> = hits_a.iter().filter(|h| h.offset == *o).map(|h| h.frame).collect();
        let most = (0..n).map(|f| hits_a.iter().filter(|h| h.offset == *o && h.frame == f).count()).max().unwrap_or(0);
        println!("  {name}: {k} hits in {} of {n} frames, at most {most} in one frame", frames.len());
    }
    all_ok &= same("hits: frame, RAM hash and registers", &hits_a, &hits_b);
    all_ok &= same("state hash after every frame", &hash_a, &hash_b);

    println!("\n2. Against a run with no breakpoints");
    b.enter()?;
    let hash_c = b.run(0, n, true)?;
    all_ok &= same("state hash after every frame, armed and not", &hash_a, &hash_c);
    match col.replays[&id].get("end-hash") {
        Some(want) if hash_a.last().map(String::as_str) == Some(want) => println!("  ok    the segment's recorded end-hash is reached"),
        Some(want) => {
            println!("  FAIL  end hash {} but the segment records {want}", hash_a.last().cloned().unwrap_or_default());
            all_ok = false;
        }
        None => println!("  (the segment records no end-hash)"),
    }

    // ---- 3: a snapshot ----------------------------------------------------------------------
    let k = n / 2;
    println!("\n3. A snapshot at frame {k}");
    b.enter()?;
    b.arm(Mode::Record, None);
    b.run(0, k, false)?;
    sp().frame = u64::MAX; // a hit while saving or loading would carry this frame number
    let snap = b.s.snapshot()?;
    sp().hits.clear();
    let tail1 = b.run(k, n, true)?;
    let hits1 = b.hits();
    sp().frame = u64::MAX;
    b.s.load_state(&snap)?;
    b.s.restore_inputs(&col, &id, k)?;
    let during: usize = sp().hits.iter().filter(|h| h.frame == u64::MAX).count();
    sp().hits.clear();
    let tail2 = b.run(k, n, true)?;
    let hits2 = b.hits();
    all_ok &= same("hits after the snapshot was loaded", &hits1, &hits2);
    all_ok &= same("state hashes after the snapshot was loaded", &tail1, &tail2);
    all_ok &= same("state hashes against the straight run", &tail1, &hash_a[k as usize..]);
    let tail_a: Vec<Hit> = hits_a.iter().filter(|h| h.frame >= k).cloned().collect();
    all_ok &= same("hits against the straight run", &hits1, &tail_a);
    println!("  {}    hits while saving or loading: {during}", if during == 0 { "ok" } else { "NOTE" });
    b.enter()?;
    b.run(0, k, false)?;
    let snap_plain = b.s.snapshot()?;
    let differing = snap.iter().zip(&snap_plain).filter(|(x, y)| x != y).count() + snap.len().abs_diff(snap_plain.len());
    println!("  {}    save state bytes, armed against not armed: {differing} differ of {}", if differing == 0 { "ok" } else { "NOTE" }, snap.len());

    // ---- 4: cost ----------------------------------------------------------------------------
    println!("\n4. Cost ({n} frames, best of 5)");
    let time = |b: &mut Bench, mode: Option<Mode>, at: Option<(i32, i64)>| -> Result<(f64, u64)> {
        let mut best = f64::MAX;
        let mut count = 0;
        for _ in 0..5 {
            b.enter()?;
            match (mode, at) {
                (Some(m), Some((r, o))) => b.arm_at(&[r], &[o], m, None),
                (Some(m), None) => b.arm(m, None),
                _ => {}
            }
            let t = Instant::now();
            b.run(0, n, false)?;
            best = best.min(t.elapsed().as_secs_f64());
            count = sp().count;
        }
        Ok((best, count))
    };
    let (t_off, _) = time(&mut b, None, None)?;
    let (t_count, c_count) = time(&mut b, Some(Mode::Count), None)?;
    let (t_rec, c_rec) = time(&mut b, Some(Mode::Record), None)?;
    println!("  not armed:                    {:8.1} frames/s", n as f64 / t_off);
    println!("  armed, the call returns:      {:8.1} frames/s  ({c_count} calls)", n as f64 / t_count);
    println!("  armed, hash RAM + registers:  {:8.1} frames/s  ({c_rec} calls)", n as f64 / t_rec);
    // The busiest instruction in the first 300 frames, to price one call.
    if let Some(prg) = prg {
        // A breakpoint that is never reached (the reset vector's bytes are data): what the
        // test on every instruction costs, with no calls.
        let (t_idle, c_idle) = time(&mut b, Some(Mode::Count), Some((prg as i32, 0x7FFC)))?;
        println!("  armed where nothing runs:     {:8.1} frames/s  ({c_idle} calls)", n as f64 / t_idle);
        b.enter()?;
        b.s.core.trace_start(8_000_000)?;
        b.run(0, 300.min(n), false)?;
        let (events, dropped) = b.s.core.trace_take();
        let mut tally: HashMap<i64, u64> = HashMap::new();
        for e in events.iter().filter(|e| e.kind == TRACE_EXEC && e.region == prg) {
            *tally.entry(e.offset).or_default() += 1;
        }
        if let Some((hot, seen)) = tally.into_iter().max_by_key(|(_, c)| *c) {
            let (t_hot, c_hot) = time(&mut b, Some(Mode::Count), Some((prg as i32, hot)))?;
            let per = (t_hot - t_idle) / c_hot.max(1) as f64 * 1e9;
            println!("  armed at the busiest instruction, PRG:0x{hot:X} ({seen} runs in 300 frames{}):", if dropped > 0 { ", trace cut short" } else { "" });
            println!("                                {:8.1} frames/s  ({c_hot} calls, {:.1} per frame)", n as f64 / t_hot, c_hot as f64 / n as f64);
            println!("  one call that returns at once costs about {per:.0} ns");
        }
    }

    // ---- 5: a write inside the call ---------------------------------------------------------
    let tick = b.offsets[0];
    let ticks = hits_a.iter().filter(|h| h.offset == tick).count() as u64;
    println!("\n5. A write inside the call: RAM[0x{poke_addr:X}] += 1 at hit {} of {}", ticks / 2, names[0]);
    if ticks < 4 {
        println!("  (too few hits to try)");
    } else {
        let poke = Poke { offset: tick, nth: ticks / 2, addr: poke_addr, seen: 0, done: None };
        b.enter()?;
        b.arm(Mode::Record, Some(poke.clone()));
        let w1 = b.run(0, n, true)?;
        let done = sp().poke.as_ref().and_then(|p| p.done);
        b.enter()?;
        b.arm(Mode::Record, Some(poke.clone()));
        let w2 = b.run(0, n, true)?;
        match done {
            None => {
                println!("  FAIL  the write was never made");
                all_ok = false;
            }
            Some((f, old, new, back)) => {
                println!("  written in frame {f}: 0x{old:02X} -> 0x{new:02X}, read back 0x{back:02X}");
                if back != new {
                    println!("  FAIL  the value read back is not the value written");
                    all_ok = false;
                }
                all_ok &= same("state hashes of two runs with the write", &w1, &w2);
                match first_diff(&w1, &hash_a) {
                    Some(i) if i as u64 == f => println!("  ok    the write takes effect in its own frame: hashes differ from frame {i} on"),
                    Some(i) => println!("  NOTE  hashes first differ from the plain run at frame {i}, the write was in frame {f}"),
                    None => {
                        println!("  FAIL  the write changed nothing");
                        all_ok = false;
                    }
                }
                // A snapshot after the write.
                let k2 = (f + 10).min(n - 1);
                b.enter()?;
                b.arm(Mode::Record, Some(poke.clone()));
                b.run(0, k2, false)?;
                let snap = b.s.snapshot()?;
                let t1 = b.run(k2, n, true)?;
                b.s.load_state(&snap)?;
                b.s.restore_inputs(&col, &id, k2)?;
                let t2 = b.run(k2, n, true)?;
                all_ok &= same(&format!("state hashes after a snapshot at frame {k2}, past the write, was loaded"), &t1, &t2);
                all_ok &= same("those against the straight run with the write", &t1, &w1[k2 as usize..]);
                // The same write between frames, before frame f.
                b.enter()?;
                b.run(0, f, false)?;
                b.s.write("RAM", poke_addr as u64, &[new])?;
                let mut fs = hash_a[..f as usize].to_vec();
                fs.extend(b.run(f, n, true)?);
                match first_diff(&w1, &fs) {
                    None => println!("  note  the same value written between frames, before frame {f}, gives the same hashes"),
                    Some(i) => println!("  note  the same value written between frames, before frame {f}, gives other hashes from frame {i} on"),
                }
            }
        }
    }

    // ---- 6: the registers -------------------------------------------------------------------
    println!("\n6. Registers at a hit");
    let wrong = hits_a.iter().filter(|h| (h.regs[5] as u16 | (h.regs[6] as u16) << 8) as i64 != 0x8000 + h.offset).count();
    println!("  {}    PC is the breakpoint's address at {} of {} hits", if wrong == 0 { "ok" } else { "FAIL" }, hits_a.len() - wrong, hits_a.len());
    all_ok &= wrong == 0;
    for h in hits_a.iter().take(3) {
        println!("  frame {:4} PRG:0x{:X}  A={:02X} X={:02X} Y={:02X} SP={:02X} P={:02X} PC={:02X}{:02X}", h.frame, h.offset, h.regs[0], h.regs[1], h.regs[2], h.regs[3], h.regs[4], h.regs[6], h.regs[5]);
    }

    println!("\n{}", if all_ok { "All checks passed." } else { "Some checks FAILED." });
    b.disarm();
    Ok(())
}
