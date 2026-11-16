//! Scratch tool for .agents/yield-parity/PLAN.md, build step 3: connector API 3, where dj
//! counts ticks itself at a breakpoint.
//!
//! Plays one segment twice and compares the tick counts: once straight, and once with the
//! connector's coverage flags cleared after every frame, as the GUI and a parity check do.
//! Under API 2 the second count came out low, because clearing the flags zeroed the count.
//! Also reads the registers and counts the opcodes the segment ran.
//!
//! Usage: tick_check <project> <rom> <core> <segment> [REGION:0xOFFSET | auto]
//!   With no address, the project's `[tick]` is used. `auto` picks the lowest instruction
//!   the segment runs in the code region, for a project that has no `[tick]`.
use anyhow::{anyhow, Result};
use decomp_dj::collection::{Collection, TickConfig};
use decomp_dj::coverage::FLAG_OPCODE;
use decomp_dj::player::{parse_region_offset, PlayOptions, Session};
use std::path::Path;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        return Err(anyhow!("usage: tick_check <project> <rom> <core> <segment> [REGION:0xOFFSET | auto]"));
    }
    let mut col = Collection::open(Path::new(&a[1]))?;
    let id = col.resolve(&a[4])?.to_string();
    let auto = a.get(5).map(String::as_str) == Some("auto");
    match a.get(5) {
        Some(_) if auto => col.config.tick = Some(TickConfig { exec: None }),
        Some(at) => col.config.tick = Some(TickConfig { exec: Some(at.clone()) }),
        None if col.config.tick.is_none() => return Err(anyhow!("the project has no [tick]: give an address, or `auto`")),
        None => {}
    }
    let mut s = Session::open_with(&col, Path::new(&a[2]), Path::new(&a[3]), false)?;
    println!("core build: {}", s.build);
    println!("segment {} \"{}\", {} frames", &id[..8], col.replays[&id].title(), col.replays[&id].frames()?);

    if auto {
        let start = s.enter_replay(&col, &id)?;
        let res = s.play(&col, &id, start, &PlayOptions::default())?;
        let i = s.core.code_region_index().ok_or_else(|| anyhow!("the connector reports no code region"))?;
        let region = &res.coverage[i];
        let off = region.flags.iter().position(|f| f & FLAG_OPCODE != 0).ok_or_else(|| anyhow!("the segment ran nothing in {}", region.name))?;
        println!("tick address: {}:0x{off:X} (the lowest instruction the segment ran)", region.name);
        s.core.set_tick_exec(Some(i), off as u64);
    } else if let Some(exec) = col.config.tick.as_ref().and_then(|t| t.exec.clone()) {
        let (r, o) = parse_region_offset(&exec)?;
        println!("tick address: {r}:0x{o:X}");
    }

    // Straight through.
    let start = s.enter_replay(&col, &id)?;
    let res = s.play(&col, &id, start, &PlayOptions::default())?;
    let plain = res.ticks;
    let opcodes: usize = res.coverage.iter().map(|r| r.flags.iter().filter(|f| *f & FLAG_OPCODE != 0).count()).sum();

    // Again, clearing the connector's flags after every frame.
    let start = s.enter_replay(&col, &id)?;
    let mut rec = s.begin(&col, &id, start, &PlayOptions::default())?;
    loop {
        let done = s.advance(&col, &mut rec, false)?;
        s.core.coverage_clear();
        if done {
            break;
        }
    }
    let cleared = s.finish(&col, rec)?.ticks;

    println!("ticks, played straight:                 {plain:?}");
    println!("ticks, flags cleared after every frame: {cleared:?}");
    println!("opcodes the segment ran: {opcodes}");
    match s.core.registers() {
        Some(r) => println!("registers: {} bytes: {}{}", r.len(), r.iter().take(24).map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "), if r.len() > 24 { " ..." } else { "" }),
        None => println!("registers: the connector gave none"),
    }
    let ok = plain.is_some() && plain == cleared && plain != Some(0) && opcodes > 0;
    println!("{}", if ok { "OK: the same count both ways, and coverage is reported." } else { "CHECK: see above." });
    Ok(())
}
