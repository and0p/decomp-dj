//! `dj xref`, `summary`, `gaps`, `function` and `frontier`: text or JSON over `decomp_dj::query`.

use anyhow::Result;
use decomp_dj::query::{GapFilter, GapSort, Page, Project, SegmentHit};
use serde::Serialize;
use std::path::Path;

use crate::QueryArgs;

fn open(dir: &Path, q: &QueryArgs) -> Result<Project> {
    Project::open(dir, q.rom.as_deref())
}

fn json<T: Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn more<T>(p: &Page<T>) {
    if let Some(n) = p.next_cursor {
        println!("  ... {} more (--cursor {n})", p.total - n);
    }
}

fn note<T>(p: &Page<T>) {
    if let Some(n) = &p.note {
        println!("{n}");
    }
}

fn hit_line(h: &SegmentHit) -> String {
    let snap = h.nearest_snapshot.map(|n| format!("snapshot at {n}")).unwrap_or_else(|| "no snapshot".into());
    let dirty = if h.counts { "" } else { "  (dirty: doesn't count)" };
    format!("{} {:<32} frame {:<6} depth {}  {snap}{dirty}", h.short, h.title, h.frame, h.depth)
}

pub fn xref(dir: &Path, what: &str, q: &QueryArgs) -> Result<()> {
    let p = open(dir, q)?;
    let x = p.xref(what, q.limit, q.cursor)?;
    if q.json {
        return json(&x);
    }
    let a = &x.address;
    let name = [a.cpu.clone(), a.symbol.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
    if let Some(n) = &a.note {
        println!("note: {n}");
    }
    if x.hits.total == 0 {
        println!("no current segment runs {} {name} (as recorded in the local database)", a.text);
        return Ok(());
    }
    println!("{} {name} first runs in:", a.text);
    for h in &x.hits.items {
        println!("  {}", hit_line(h));
    }
    more(&x.hits);
    Ok(())
}

pub fn summary(dir: &Path, q: &QueryArgs) -> Result<()> {
    let mut p = open(dir, q)?;
    let s = p.summary()?;
    if q.json {
        return json(&s);
    }
    if s.symbols {
        println!("instructions  {:>6} / {:<6} {:.1}%", s.instructions.covered, s.instructions.total, s.instructions.percent);
        println!("blocks        {:>6} / {:<6} {:.1}%", s.blocks.covered, s.blocks.total, s.blocks.percent);
        println!("functions     {:>6} / {:<6} {:.1}%", s.functions.covered, s.functions.total, s.functions.percent);
        println!("by play alone {:>6} / {:<6} {:.1}%", s.by_play_alone.covered, s.by_play_alone.total, s.by_play_alone.percent);
        println!("never entered {}, partly covered {}", s.functions_never_entered, s.functions_partly_covered);
    } else {
        println!("opcode bytes run: {} (no symbol map)", s.instructions.covered);
    }
    if s.unreachable_instructions + s.data_bytes > 0 {
        println!("left out (annotations.toml): {} unreachable, {} data bytes, {} whole functions", s.unreachable_instructions, s.data_bytes, s.annotated_functions);
    }
    for w in &s.annotation_warnings {
        println!("warning: {w}");
    }
    println!("{} segments ({} dirty and {} unplayed left out)", s.segments, s.segments_left_out.dirty, s.segments_left_out.unplayed);
    for t in &s.top_segments {
        println!("  {t}");
    }
    Ok(())
}

pub fn gaps(dir: &Path, sort: GapSort, only: GapFilter, q: &QueryArgs) -> Result<()> {
    let mut p = open(dir, q)?;
    if !p.has_symbols()? {
        let g = p.uncovered_ranges(sort, q.limit, q.cursor)?;
        if q.json {
            return json(&g);
        }
        note(&g);
        println!("  {:<24} {:>7}", "range", "bytes");
        for r in &g.items {
            let range = format!("{}-{}", r.at, r.end.split_once(':').map_or(r.end.as_str(), |(_, o)| o));
            println!("  {:<24} {:>7}", range, r.bytes);
        }
        more(&g);
        return Ok(());
    }
    let g = p.gaps(sort, only, q.limit, q.cursor)?;
    if q.json {
        return json(&g);
    }
    println!("  {:<28} {:>7} {:>9} {:>7}  {}", "function", "missing", "of instrs", "blocks", "skipped by a covered branch");
    for r in &g.items {
        let name = format!("{} {}", r.cpu.clone().unwrap_or_else(|| r.at.clone()), r.function);
        let near = if r.skipped_by_covered_branch > 0 { format!("{} (try {})", r.skipped_by_covered_branch, r.best_frontier_block.clone().unwrap_or_default()) } else if r.entered { "-".into() } else { "never entered".into() };
        println!("  {:<28} {:>7} {:>9} {:>3}/{:<3}  {near}", name.chars().take(28).collect::<String>(), r.uncovered_instructions, r.instructions, r.uncovered_blocks, r.blocks);
    }
    more(&g);
    Ok(())
}

pub fn function(dir: &Path, what: &str, q: &QueryArgs) -> Result<()> {
    let mut p = open(dir, q)?;
    let f = p.function(what, q.limit)?;
    if q.json {
        return json(&f);
    }
    if let Some(n) = &f.note {
        println!("{n}");
    }
    println!("{} {} ({}, {} bytes): {} of {} instructions run", f.cpu.clone().unwrap_or_default(), f.name, f.at, f.size, f.instructions_covered, f.instructions);
    for b in &f.blocks {
        let first = b.first_run.as_ref().map(|r| format!("first in {} {} frame {}", r.short, r.title, r.frame)).unwrap_or_default();
        println!("  {} {:<12} {:>3} instrs  {:<8} -> {:<16} {first}", if b.covered { "■" } else { "□" }, b.cpu.clone().unwrap_or_else(|| b.at.clone()), b.instructions, if b.covered { "covered" } else { "MISSING" }, b.next.join(" "));
    }
    if !f.callers.is_empty() {
        println!("callers:");
        for c in &f.callers {
            println!("  {} {} in {}{}", c.kind, c.cpu.clone().unwrap_or_else(|| c.at.clone()), c.function, if c.covered { "" } else { "  (never runs)" });
        }
    }
    if f.entered_by.total > 0 {
        println!("entered by:");
        for h in &f.entered_by.items {
            println!("  {}", hit_line(h));
        }
        more(&f.entered_by);
    } else {
        println!("never entered");
    }
    Ok(())
}

pub fn frontier(dir: &Path, what: Option<&str>, q: &QueryArgs) -> Result<()> {
    let mut p = open(dir, q)?;
    let e = p.frontier(what, q.limit, q.cursor)?;
    if q.json {
        return json(&e);
    }
    note(&e);
    for x in &e.items {
        println!("{} {} -> {} in {} ({} uncovered instructions beyond it)", x.kind, x.branch, x.uncovered, x.function, x.uncovered_instructions);
        for h in &x.reached_by {
            println!("    {}", hit_line(h));
        }
        if x.reached_by.is_empty() {
            println!("    (no segment's first-hit record for it)");
        }
    }
    more(&e);
    Ok(())
}

/// `dj check-connector`: prints what the check found and fails when the flags disagree with
/// the decoder.
pub fn check_connector(dir: &Path, q: &QueryArgs) -> Result<()> {
    let mut p = open(dir, q)?;
    let r = p.check_connector()?;
    if q.json {
        json(&r)?;
    } else {
        println!("instructions checked (ran, inside symbol-map functions): {}", r.instructions_checked);
        println!("operand flags: {}", if r.operand_flags { "yes" } else { "none set: only opcodes checked, operand bytes come from instruction lengths" });
        let list = |label: &str, v: &[u64]| {
            let shown: Vec<String> = v.iter().take(q.limit).map(|o| format!("PRG:{o:#x}")).collect();
            let more = if v.len() > shown.len() { format!(" ... {} more", v.len() - shown.len()) } else { String::new() };
            println!("{label}: {}{}{more}", v.len(), if shown.is_empty() { String::new() } else { format!("  {}", shown.join(" ")) });
        };
        list("opcode flag where the decoder starts no instruction", &r.opcode_not_at_start);
        list("operand flag on a byte that isn't an operand of an instruction that ran", &r.operand_unexpected);
        list("operand byte of an instruction that ran, without the operand flag", &r.operand_missing);
        println!("opcodes run in both widths of a mode pair: {}", r.both_widths);
    }
    let bad = r.opcode_not_at_start.len() + r.operand_unexpected.len() + r.operand_missing.len();
    if bad > 0 {
        anyhow::bail!("{bad} bytes where the connector's flags and the decoder disagree");
    }
    Ok(())
}
