//! Scratch tool for the Thwaite decomp's milestone 0 (.agents/THWAITE_DECOMP_PLAN.md): checks
//! the decomp's breakpoint table against the original, on every segment of the project.
//!
//! Needs a Mesen core with the version 3 connector (`dj_set_breakpoints`,
//! `dj_read_registers`; connectors/mesen/build.sh), and the table the decomp generates
//! (examples/thwaite/.decomp/src/data/breakpoints.tsv). For each segment, played from
//! power-on with every breakpoint armed, it reports:
//!   1. how often each breakpoint is hit, and the most hits in one frame;
//!   2. frames with no wait exit (lag frames, boot) and frames with more than one;
//!   3. where the PC is at each frame end: in a wait loop's spin, or not;
//!   4. counter reads that saw a different `nmis` from the step's wait exit (an NMI mid-step);
//!   5. for a root segment, the first frame with a wait exit (how long boot is).
//!
//! Usage: thwaite_breaks <project> <rom> <core> <breakpoints.tsv>
use anyhow::{anyhow, Result};
use decomp_dj::collection::Collection;
use decomp_dj::libretro::MEMORY_SYSTEM_RAM;
use decomp_dj::player::Session;
use std::collections::BTreeMap;
use std::path::Path;

/// `struct dj_break` and `dj_break_fn` (docs/connectors.md).
#[repr(C)]
struct DjBreak {
    region: i32,
    offset: i64,
}
type BreakFn = unsafe extern "C" fn(*mut std::ffi::c_void, i32, i64) -> i32;
type SetBreaks = unsafe extern "C" fn(*const DjBreak, usize, Option<BreakFn>, *mut std::ffi::c_void);

const NMIS: usize = 0x30;

struct Bp {
    kind: String,
    name: String,
    offset: i64,
    spin: (i64, i64),
}

struct Hit {
    frame: u64,
    offset: i64,
    nmis: u8,
}

struct Rec {
    ram: *const u8,
    frame: u64,
    hits: Vec<Hit>,
}

static mut REC: Rec = Rec { ram: std::ptr::null(), frame: 0, hits: Vec::new() };

fn rec() -> &'static mut Rec {
    unsafe { &mut *std::ptr::addr_of_mut!(REC) }
}

unsafe extern "C" fn on_break(_user: *mut std::ffi::c_void, _region: i32, offset: i64) -> i32 {
    let r = rec();
    let nmis = unsafe { *r.ram.add(NMIS) };
    r.hits.push(Hit { frame: r.frame, offset, nmis });
    0
}

fn read_table(path: &Path) -> Result<Vec<Bp>> {
    let mut out = vec![];
    for line in std::fs::read_to_string(path)?.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let hex = |s: &str| i64::from_str_radix(s.trim_start_matches("PRG:").trim_start_matches("0x"), 16);
        out.push(Bp { kind: f[1].into(), name: f[2].into(), offset: hex(f[3])?, spin: (hex(f[4])?, hex(f[5])?) });
    }
    Ok(out)
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        return Err(anyhow!("usage: thwaite_breaks <project> <rom> <core> <breakpoints.tsv>"));
    }
    let col = Collection::open(Path::new(&a[1]))?;
    let bps = read_table(Path::new(&a[4]))?;
    let mut s = Session::open_with(&col, Path::new(&a[2]), Path::new(&a[3]), false)?;
    let set: SetBreaks = {
        let p = s.core.proc("dj_set_breakpoints");
        if p.is_null() {
            return Err(anyhow!("this core has no dj_set_breakpoints: rebuild it (connectors/mesen/build.sh)"));
        }
        unsafe { std::mem::transmute(p) }
    };
    let prg = s.core.coverage_region_index("PRG").ok_or_else(|| anyhow!("no PRG region"))? as i32;
    rec().ram = s.core.memory(MEMORY_SYSTEM_RAM).ok_or_else(|| anyhow!("no system RAM"))?.as_ptr();
    let list: Vec<DjBreak> = bps.iter().map(|b| DjBreak { region: prg, offset: b.offset }).collect();
    let waits: Vec<&Bp> = bps.iter().filter(|b| b.kind == "wait").collect();
    let name_of = |o: i64| bps.iter().find(|b| b.offset == o).map(|b| b.name.as_str()).unwrap_or("?");

    let mut ids: Vec<String> = col.replays.keys().cloned().collect();
    ids.sort_by_key(|id| col.replays[id].title().to_string());
    let mut totals: BTreeMap<&str, u64> = BTreeMap::new();
    for id in &ids {
        let n = col.replays[id].frames()?;
        let root = col.replays[id].parent().is_none();
        unsafe { set(std::ptr::null(), 0, None, std::ptr::null_mut()) };
        s.enter_replay(&col, id)?;
        rec().hits.clear();
        // Replaces dj's own list (the [tick] address) until the segment ends.
        unsafe { set(list.as_ptr(), list.len(), Some(on_break), std::ptr::null_mut()) };
        let mut end_pc = vec![];
        for f in 0..n {
            rec().frame = f;
            s.step(&col, id, f, false)?;
            let r = s.core.registers().ok_or_else(|| anyhow!("the connector gives no registers"))?;
            end_pc.push((r[5] as i64 | (r[6] as i64) << 8) - 0x8000);
        }
        unsafe { set(std::ptr::null(), 0, None, std::ptr::null_mut()) };
        let hits = std::mem::take(&mut rec().hits);

        println!("\n== {} \"{}\" ({n} frames{})", &id[..8], col.replays[id].title(), if root { ", from power-on" } else { "" });
        // 1. per breakpoint
        for b in &bps {
            let mine: Vec<&Hit> = hits.iter().filter(|h| h.offset == b.offset).collect();
            if mine.is_empty() {
                continue;
            }
            let mut per: BTreeMap<u64, u64> = BTreeMap::new();
            for h in &mine {
                *per.entry(h.frame).or_default() += 1;
            }
            println!("  {:30} {:6} hits in {:5} frames, at most {} in one", b.name, mine.len(), per.len(), per.values().max().unwrap());
            *totals.entry(b.name.as_str()).or_default() += mine.len() as u64;
        }
        // 2. frames by wait exits
        let mut per_frame = vec![0u32; n as usize];
        for h in hits.iter().filter(|h| waits.iter().any(|w| w.offset == h.offset)) {
            per_frame[h.frame as usize] += 1;
        }
        let first = per_frame.iter().position(|c| *c > 0);
        if root {
            println!("  boot: first wait exit in frame {}", first.map(|f| f.to_string()).unwrap_or("none".into()));
        }
        let from = first.unwrap_or(n as usize);
        let none: Vec<usize> = (from..n as usize).filter(|f| per_frame[*f] == 0).collect();
        let many: Vec<usize> = (from..n as usize).filter(|f| per_frame[*f] > 1).collect();
        println!("  frames after the first wait exit with none: {} {:?}", none.len(), &none[..none.len().min(12)]);
        println!("  frames with more than one: {} {:?}", many.len(), &many[..many.len().min(12)]);
        // 3. PC at frame ends
        let spin_of = |pc: i64| waits.iter().find(|w| pc >= w.spin.0 && pc < w.spin.1).map(|w| w.name.as_str());
        let mut where_: BTreeMap<String, u64> = BTreeMap::new();
        let mut outside = vec![];
        for (f, pc) in end_pc.iter().enumerate().skip(from) {
            match spin_of(*pc) {
                Some(w) => *where_.entry(w.to_string()).or_default() += 1,
                None => outside.push((f, *pc)),
            }
        }
        println!("  frame ends in a spin: {:?}", where_);
        println!("  frame ends outside one: {} {:?}", outside.len(),
                 outside.iter().take(8).map(|(f, pc)| format!("{f}@PRG:0x{pc:X}")).collect::<Vec<_>>());
        // 4. counter reads that saw another value than the step's wait exit, and input reads
        // in another frame than the step's wait exit
        let kind_of = |o: i64| bps.iter().find(|b| b.offset == o).map(|b| b.kind.as_str()).unwrap_or("");
        let mut last_wait: Option<(u8, u64)> = None;
        let (mut moved, mut late_input) = (vec![], vec![]);
        for h in &hits {
            match kind_of(h.offset) {
                "wait" => last_wait = Some((h.nmis, h.frame)),
                "nmis" => {
                    if let Some((n, _)) = last_wait {
                        if h.nmis != n {
                            moved.push(format!("{}@{}", name_of(h.offset), h.frame));
                        }
                    }
                }
                "input" => {
                    if let Some((_, f)) = last_wait {
                        if h.frame != f {
                            late_input.push(format!("{}@{} (step from {f})", name_of(h.offset), h.frame));
                        }
                    }
                }
                _ => {}
            }
        }
        println!("  counter reads that differ from the step's wait exit: {} {:?}", moved.len(), &moved[..moved.len().min(8)]);
        println!("  input reads in a later frame than the step's wait exit: {} {:?}", late_input.len(), &late_input[..late_input.len().min(8)]);
    }
    println!("\n== totals over every segment");
    for (k, v) in &totals {
        println!("  {k:30} {v}");
    }
    let never: Vec<&str> = bps.iter().map(|b| b.name.as_str()).filter(|n| !totals.contains_key(n)).collect();
    println!("  never hit: {never:?}");
    Ok(())
}
