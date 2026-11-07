//! Local coverage database and reports (docs/coverage.md).

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::analysis::{self, AnalyzedFunction};
use crate::collection::Collection;
use crate::cache;
use crate::collection::Status;

pub const FLAG_OPCODE: u8 = 0x01;

// ---- report -------------------------------------------------------------------

#[derive(Serialize)]
pub struct Totals {
    pub functions: usize,
    pub functions_entered: usize,
    pub blocks: usize,
    pub blocks_covered: usize,
    pub instructions: usize,
    pub instructions_covered: usize,
    pub played_only_instructions_covered: usize,
    pub executed_bytes_by_region: BTreeMap<String, usize>,
    pub executed_outside_symbols: usize,
    /// Instructions inside unreachable.txt ranges (left out of the counts above).
    pub unreachable_instructions: usize,
    /// Segments left out because they are dirty, or have not been played yet.
    pub dirty_segments: usize,
    pub unplayed_segments: usize,
    /// Segments with unsaved changes, and drafts: left out until saved.
    pub unsaved_segments: usize,
    /// Units (instructions, or opcode bytes) only unsaved segments run: they count once saved.
    pub unsaved_new: usize,
    /// Units only dirty segments ran, the last time each played: they count once those
    /// segments are clean again, if they still run them.
    pub if_cleaned: usize,
}

#[derive(Serialize)]
pub struct ReplayRow {
    pub id: String,
    pub title: String,
    pub depth: usize,
    pub poked: bool,
    pub ticks: Option<i64>,
    /// Instructions (or opcode bytes, without symbols) run during this replay's own frames.
    pub own: usize,
    /// ... that no ancestor ran.
    pub new_vs_ancestors: usize,
    /// ... that no other replay ran.
    pub unique: usize,
}

#[derive(Serialize)]
pub struct FunctionRow {
    pub name: String,
    pub offset: String,
    pub cpu_addr: Option<String>,
    pub instructions: usize,
    pub instructions_covered: usize,
    pub blocks: usize,
    pub blocks_covered: usize,
    pub uncovered_blocks: Vec<String>,
}

#[derive(Serialize)]
pub struct Report {
    /// The setup (ROM, core and options) the coverage was recorded with.
    pub setup: String,
    pub symbols: bool,
    pub totals: Totals,
    pub replays: Vec<ReplayRow>,
    pub smallest_set: Vec<String>,
    pub functions: Vec<FunctionRow>,
    /// What the report was worked out from, for views that need more than the rows (the GUI).
    #[serde(skip)]
    pub detail: Detail,
}

/// The raw material behind a report: the analyzed functions (unreachable instructions
/// removed, in the same order as `Report::functions`) and the union of the opcode flags
/// of every segment that counts.
#[derive(Default)]
pub struct Detail {
    pub analyzed: Vec<AnalyzedFunction>,
    /// Instructions of each analyzed function that unreachable.txt removed.
    pub unreachable_per_function: Vec<usize>,
    /// PRG flags of every segment that counts, OR'd together.
    pub prg: Vec<u8>,
    /// PRG flags of unsaved segments (and drafts) that are clean, OR'd together.
    pub unsaved_prg: Vec<u8>,
    /// PRG flags of dirty segments from their last recorded play, OR'd together.
    pub dirty_prg: Vec<u8>,
    /// Each unsaved segment's (or draft's) PRG flags, as in `unsaved_prg`.
    pub unsaved_per_segment: BTreeMap<String, Vec<u8>>,
    /// Segments that count, with their PRG flags.
    pub per_segment_prg: BTreeMap<String, Vec<u8>>,
    /// PRG ranges from unreachable.txt (inclusive).
    pub unreachable: Vec<(u64, u64)>,
}

impl Detail {
    /// Whether PRG offset `off` ran as an opcode in any segment that counts.
    pub fn hit(&self, off: u64) -> bool {
        self.prg.get(off as usize).is_some_and(|b| b & FLAG_OPCODE != 0)
    }

    /// Whether `off` ran in an unsaved segment that is clean.
    pub fn unsaved_hit(&self, off: u64) -> bool {
        self.unsaved_prg.get(off as usize).is_some_and(|b| b & FLAG_OPCODE != 0)
    }

    /// Whether `off` ran in a dirty segment's last recorded play.
    pub fn dirty_hit(&self, off: u64) -> bool {
        self.dirty_prg.get(off as usize).is_some_and(|b| b & FLAG_OPCODE != 0)
    }

    pub fn is_unreachable(&self, off: u64) -> bool {
        self.unreachable.iter().any(|(a, b)| off >= *a && off <= *b)
    }
}

fn union(a: &mut Vec<u8>, b: &[u8]) {
    if a.len() < b.len() {
        a.resize(b.len(), 0);
    }
    for (x, y) in a.iter_mut().zip(b) {
        *x |= *y;
    }
}

fn opcode_set(flags: &[u8]) -> Vec<bool> {
    flags.iter().map(|f| f & FLAG_OPCODE != 0).collect()
}

/// Coverage of the collection as it is now, from the local database: every segment that
/// isn't dirty and has been played with its current content counts (docs/coverage.md).
/// `setup` picks the setup (ROM, core and options) to report on; by default, the one played last.
pub fn build_report(col: &Collection, conn: &Connection, rom: Option<&[u8]>, setup: Option<&str>) -> Result<Report> {
    build_report_with(col, conn, rom, setup, &std::collections::HashSet::new())
}

/// `build_report` for an editor: `unsaved` names segments with changes not saved yet. Like
/// drafts, they don't count; what they (or dirty segments, from their last recorded play)
/// would add is reported apart (`Totals::unsaved_new`, `Totals::if_cleaned`).
pub fn build_report_with(col: &Collection, conn: &Connection, rom: Option<&[u8]>, setup: Option<&str>, unsaved: &std::collections::HashSet<String>) -> Result<Report> {
    let setup: String = match setup {
        Some(s) => s.to_string(),
        None => conn
            .query_row("SELECT setup FROM contents ORDER BY played_at DESC LIMIT 1", [], |r| r.get(0))
            .context("nothing has been played into the local database yet; run `dj clean` first")?,
    };
    let keys = cache::current_keys(conn, col, &setup)?;
    let mut cov: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    let mut ticks: HashMap<String, Option<i64>> = HashMap::new();
    let (mut dirty_segments, mut unplayed_segments) = (0, 0);
    let (mut unsaved_prg, mut dirty_prg): (Vec<u8>, Vec<u8>) = (vec![], vec![]);
    let mut unsaved_segments = 0;
    let mut unsaved_per_segment: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut last_run = conn.prepare_cached("SELECT c.key, c.body FROM segment_runs s JOIN contents c ON c.key = s.content WHERE s.segment_id = ?1 AND c.setup = ?2")?;
    for (id, (key, status)) in &keys {
        // Unsaved first (drafts too, which have no end hash until played, so are dirty):
        // whatever they run is new until saved, yellow-green, whether or not they are clean.
        let is_unsaved = col.is_draft(id) || unsaved.contains(id);
        if is_unsaved || *status == Status::Dirty {
            if is_unsaved {
                unsaved_segments += 1;
            } else {
                dirty_segments += 1;
            }
            // Its current content if that was played; otherwise what it ran the last time it
            // played with these lines, whatever it started from.
            let mut regions = match key {
                Some(k) => cache::coverage_of(conn, k)?,
                None => HashMap::new(),
            };
            if regions.is_empty() {
                let r = &col.replays[id];
                let body = cache::body_hash(r, &crate::player::devices_at_start(col, id)?)?;
                let last: Option<(String, String)> = last_run.query_row(rusqlite::params![id, setup], |row| Ok((row.get(0)?, row.get(1)?))).ok();
                if let Some((k, _)) = last.filter(|(_, b)| *b == body) {
                    regions = cache::coverage_of(conn, &k)?;
                }
            }
            if let Some(p) = regions.get("PRG") {
                union(if is_unsaved { &mut unsaved_prg } else { &mut dirty_prg }, p);
                if is_unsaved {
                    unsaved_per_segment.insert(id.clone(), p.clone());
                }
            }
            continue;
        }
        let Some(key) = key else {
            unplayed_segments += 1;
            continue;
        };
        let regions = cache::coverage_of(conn, key)?;
        if regions.is_empty() {
            unplayed_segments += 1;
            continue;
        }
        cov.insert(id.clone(), regions.into_iter().collect());
        let t: Option<Option<i64>> = conn.query_row("SELECT ticks FROM contents WHERE key = ?1", [key], |r| r.get(0)).ok();
        ticks.insert(id.clone(), t.flatten());
    }
    if cov.is_empty() && unsaved_prg.is_empty() && dirty_prg.is_empty() {
        bail!("no segment has coverage recorded for its current content (was the core built with a coverage connector?); run `dj clean`");
    }
    let unreachable = cache::load_unreachable(col)?;
    let is_unreachable = |off: u64| unreachable.iter().any(|u| u.region == "PRG" && off >= u.start && off <= u.end);

    // Symbols -> analyzed functions (PRG only for now, 6502). With several symbol maps, the
    // first one listed wins where two describe the same bytes.
    let mut analyzed: Vec<AnalyzedFunction> = vec![];
    let mut taken: Vec<(u64, u64)> = vec![];
    for s in &col.config.symbols {
        let funcs = analysis::load_symbols(&col.root, s, rom)?;
        let funcs: Vec<_> = funcs.into_iter().filter(|f| f.region == "PRG" && !taken.iter().any(|(a, b)| f.offset < *b && f.offset + f.size > *a)).collect();
        if funcs.is_empty() {
            continue;
        }
        taken.extend(funcs.iter().map(|f| (f.offset, f.offset + f.size)));
        let Some(rom) = rom else { bail!("symbols need the ROM to disassemble") };
        let header = s.rom_header.unwrap_or_else(|| analysis::default_rom_header(&s.format, Some(rom))) as usize;
        let prg = &rom[header.min(rom.len())..];
        analyzed.extend(analysis::analyze_6502(&funcs, prg));
    }
    analyzed.sort_by_key(|f| f.func.offset);
    let symbols = !analyzed.is_empty();
    let mut unreachable_instructions = 0;
    let mut unreachable_per_function = vec![];
    for f in &mut analyzed {
        let before = f.instructions.len();
        f.instructions.retain(|i| !is_unreachable(i.offset));
        f.blocks.retain(|b| !is_unreachable(b.offset));
        unreachable_instructions += before - f.instructions.len();
        unreachable_per_function.push(before - f.instructions.len());
    }

    // Units of coverage: instruction offsets in PRG when we have symbols, else opcode bytes.
    let units: Vec<u64> = if symbols {
        analyzed.iter().flat_map(|f| f.instructions.iter().map(|i| i.offset)).collect()
    } else {
        let max = cov.values().filter_map(|m| m.get("PRG")).map(|f| f.len()).chain([unsaved_prg.len(), dirty_prg.len()]).max().unwrap_or(0);
        (0..max as u64).collect()
    };
    let hit = |flags: Option<&Vec<u8>>, off: u64| flags.and_then(|f| f.get(off as usize)).map(|b| b & FLAG_OPCODE != 0).unwrap_or(false);

    // Unions.
    let mut all_prg: Vec<u8> = vec![];
    let mut played_prg: Vec<u8> = vec![];
    let mut by_region: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (id, regions) in &cov {
        for (name, flags) in regions {
            union(by_region.entry(name.clone()).or_default(), flags);
        }
        if let Some(p) = regions.get("PRG") {
            union(&mut all_prg, p);
            if !col.replays.get(id).map(|r| r.is_poked()).unwrap_or(false) {
                union(&mut played_prg, p);
            }
        }
    }
    let executed_bytes_by_region = by_region.iter().map(|(k, v)| (k.clone(), opcode_set(v).iter().filter(|b| **b).count())).collect();
    let in_symbols: std::collections::HashSet<u64> = units.iter().copied().collect();
    let executed_outside_symbols = if symbols {
        opcode_set(&all_prg).iter().enumerate().filter(|(i, b)| **b && !in_symbols.contains(&(*i as u64))).count()
    } else {
        0
    };

    // Per-replay rows.
    let unit_hits = |flags: Option<&Vec<u8>>| -> Vec<bool> { units.iter().map(|u| hit(flags, *u)).collect() };
    let per_replay: HashMap<&str, Vec<bool>> = cov.iter().map(|(id, m)| (id.as_str(), unit_hits(m.get("PRG")))).collect();
    let mut replays = vec![];
    let empty = vec![false; units.len()];
    // How many segments run each unit: a unit is unique to a segment when only it runs it.
    let mut hit_count = vec![0u32; units.len()];
    for v in per_replay.values() {
        for (n, h) in hit_count.iter_mut().zip(v) {
            *n += *h as u32;
        }
    }
    for (id, depth) in col.tree_order() {
        if col.is_draft(id) {
            continue;
        }
        let mine = per_replay.get(id).unwrap_or(&empty);
        let mut anc = vec![false; units.len()];
        for a in col.chain(id)?.iter().rev().skip(1) {
            if let Some(v) = per_replay.get(a) {
                for (x, y) in anc.iter_mut().zip(v) {
                    *x |= *y;
                }
            }
        }
        let r = &col.replays[id];
        replays.push(ReplayRow {
            id: id.to_string(),
            title: r.title().to_string(),
            depth,
            poked: r.is_poked(),
            ticks: ticks.get(id).copied().flatten(),
            own: mine.iter().filter(|b| **b).count(),
            new_vs_ancestors: mine.iter().zip(&anc).filter(|(m, a)| **m && !**a).count(),
            unique: mine.iter().zip(&hit_count).filter(|(m, n)| **m && **n == 1).count(),
        });
    }

    // Smallest set of replays (greedy): each pick brings its whole chain.
    let chain_cov: HashMap<&str, Vec<bool>> = per_replay
        .keys()
        .map(|id| {
            let mut v = vec![false; units.len()];
            for c in col.chain(id).unwrap_or_default() {
                if let Some(p) = per_replay.get(c) {
                    for (x, y) in v.iter_mut().zip(p) {
                        *x |= *y;
                    }
                }
            }
            (*id, v)
        })
        .collect();
    let mut got = vec![false; units.len()];
    let mut smallest_set = vec![];
    loop {
        let best = chain_cov
            .iter()
            .map(|(id, v)| (v.iter().zip(&got).filter(|(a, g)| **a && !**g).count(), *id))
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(a.1)));
        match best {
            Some((gain, id)) if gain > 0 => {
                for (g, v) in got.iter_mut().zip(&chain_cov[id]) {
                    *g |= *v;
                }
                smallest_set.push(id.to_string());
            }
            _ => break,
        }
    }

    // Function rows.
    let all_flags = Some(&all_prg);
    let mut functions = vec![];
    let (mut nf, mut nfe, mut nb, mut nbc, mut ni, mut nic, mut npi) = (0, 0, 0, 0, 0, 0, 0);
    for f in &analyzed {
        let icov = f.instructions.iter().filter(|i| hit(all_flags, i.offset)).count();
        let pcov = f.instructions.iter().filter(|i| hit(Some(&played_prg), i.offset)).count();
        let mut uncovered = vec![];
        let mut bcov = 0;
        for b in &f.blocks {
            if hit(all_flags, b.offset) {
                bcov += 1;
            } else {
                uncovered.push(match b.cpu_addr {
                    Some(c) => format!("${c:04X}"),
                    None => format!("PRG:{:#x}", b.offset),
                });
            }
        }
        nf += 1;
        if icov > 0 {
            nfe += 1;
        }
        nb += f.blocks.len();
        nbc += bcov;
        ni += f.instructions.len();
        nic += icov;
        npi += pcov;
        functions.push(FunctionRow {
            name: f.func.name.clone(),
            offset: format!("{:#x}", f.func.offset),
            cpu_addr: f.func.cpu_addr.map(|c| format!("${c:04X}")),
            instructions: f.instructions.len(),
            instructions_covered: icov,
            blocks: f.blocks.len(),
            blocks_covered: bcov,
            uncovered_blocks: uncovered,
        });
    }
    // What would count once saved, and once the dirty segments are clean (and saved).
    let unsaved_new = units.iter().filter(|u| hit(Some(&unsaved_prg), **u) && !hit(all_flags, **u)).count();
    let if_cleaned = units.iter().filter(|u| hit(Some(&dirty_prg), **u) && !hit(all_flags, **u) && !hit(Some(&unsaved_prg), **u)).count();
    if !symbols {
        ni = units.len();
        nic = opcode_set(&all_prg).iter().filter(|b| **b).count();
        npi = opcode_set(&played_prg).iter().filter(|b| **b).count();
    }

    Ok(Report {
        setup,
        symbols,
        totals: Totals {
            functions: nf,
            functions_entered: nfe,
            blocks: nb,
            blocks_covered: nbc,
            instructions: ni,
            instructions_covered: nic,
            played_only_instructions_covered: npi,
            executed_bytes_by_region,
            executed_outside_symbols,
            unreachable_instructions,
            dirty_segments,
            unplayed_segments,
            unsaved_segments,
            unsaved_new,
            if_cleaned,
        },
        replays,
        smallest_set,
        functions,
        detail: Detail {
            analyzed,
            unreachable_per_function,
            per_segment_prg: cov.iter().filter_map(|(id, m)| m.get("PRG").map(|p| (id.clone(), p.clone()))).collect(),
            prg: all_prg,
            unsaved_prg,
            dirty_prg,
            unsaved_per_segment,
            unreachable: unreachable.iter().filter(|u| u.region == "PRG").map(|u| (u.start, u.end)).collect(),
        },
    })
}

/// Where code first runs: one row per current segment whose recorded play ran any ROM
/// byte of `region` in `start..=end` as an opcode, with the first frame it did.
pub struct FirstHit {
    pub id: String,
    pub frame: u64,
    pub offset: u64,
    pub status: Status,
    /// Segments above it, so 0 for a root segment.
    pub depth: usize,
}

/// First hits of `region` `start..=end` in the segments as they are now, for `setup`. Sorted so
/// the most useful comes first: segments that count (not dirty) before dirty ones, then
/// shallower segments, then earlier frames.
pub fn first_hits_in(col: &Collection, conn: &Connection, setup: &str, region: &str, start: u64, end: u64) -> Result<Vec<FirstHit>> {
    let keys = cache::current_keys(conn, col, setup)?;
    let by_key: HashMap<&str, (&str, Status)> = keys.iter().filter_map(|(id, (k, st))| k.as_deref().map(|k| (k, (id.as_str(), *st)))).collect();
    let mut st = conn.prepare_cached("SELECT content, rom_offset, frame FROM first_hits WHERE region = ?1 AND rom_offset BETWEEN ?2 AND ?3")?;
    let rows = st.query_map(rusqlite::params![region, start as i64, end as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64)))?;
    let mut best: HashMap<&str, FirstHit> = HashMap::new();
    for row in rows {
        let (content, offset, frame) = row?;
        let Some((id, status)) = by_key.get(content.as_str()) else { continue };
        let e = best.entry(id).or_insert(FirstHit { id: id.to_string(), frame, offset, status: *status, depth: col.chain(id).map(|c| c.len() - 1).unwrap_or(0) });
        if frame < e.frame {
            e.frame = frame;
            e.offset = offset;
        }
    }
    let mut out: Vec<FirstHit> = best.into_values().collect();
    out.sort_by_key(|h| (h.status == Status::Dirty, h.depth, h.frame, h.id.clone()));
    Ok(out)
}

fn pct(a: usize, b: usize) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.1}%", a as f64 * 100.0 / b as f64)
    }
}

pub fn print_report(rep: &Report, col: &Collection) {
    let t = &rep.totals;
    println!("Coverage of the current segments (setup {})", rep.setup);
    if t.dirty_segments + t.unplayed_segments > 0 {
        println!("  left out: {} dirty, {} not played with their current content", t.dirty_segments, t.unplayed_segments);
    }
    if t.unsaved_segments > 0 {
        println!("  left out: {} drafts, +{} units they would add once saved", t.unsaved_segments, t.unsaved_new);
    }
    if t.if_cleaned > 0 {
        println!("  +{} units the dirty segments ran when last played: they count once those are clean", t.if_cleaned);
    }
    if t.unreachable_instructions > 0 {
        println!("  unreachable (unreachable.txt): {} instructions, not counted", t.unreachable_instructions);
    }
    if rep.symbols {
        println!("  functions entered    {:>6} / {:<6} {}", t.functions_entered, t.functions, pct(t.functions_entered, t.functions));
        println!("  basic blocks         {:>6} / {:<6} {}", t.blocks_covered, t.blocks, pct(t.blocks_covered, t.blocks));
        println!("  instructions         {:>6} / {:<6} {}", t.instructions_covered, t.instructions, pct(t.instructions_covered, t.instructions));
        println!("  ... by play alone    {:>6} / {:<6} {}", t.played_only_instructions_covered, t.instructions, pct(t.played_only_instructions_covered, t.instructions));
        if t.executed_outside_symbols > 0 {
            println!("  executed PRG bytes outside known functions: {}", t.executed_outside_symbols);
        }
    } else {
        println!("  (no symbol map: counting executed opcode bytes)");
    }
    for (region, n) in &t.executed_bytes_by_region {
        println!("  opcode bytes run in {region:<5} {n}");
    }
    println!();
    println!("  {:<44} {:>7} {:>7} {:>7} {:>7}", "replay", "own", "new", "unique", "ticks");
    for r in &rep.replays {
        let name = format!("{}{} {}", "  ".repeat(r.depth), &r.id[..8], r.title);
        let name: String = name.chars().take(44).collect();
        let ticks = r.ticks.map(|t| t.to_string()).unwrap_or_else(|| "-".into());
        println!("  {:<44} {:>7} {:>7} {:>7} {:>7}{}", name, r.own, r.new_vs_ancestors, r.unique, ticks, if r.poked { "  poked" } else { "" });
    }
    println!();
    let titles: Vec<String> = rep.smallest_set.iter().map(|id| format!("{} {}", &id[..8], col.replays[id].title())).collect();
    println!("  smallest set keeping all coverage ({} of {}): {}", rep.smallest_set.len(), rep.replays.len(), titles.join("; "));
    if rep.symbols {
        let never: Vec<&FunctionRow> = rep.functions.iter().filter(|f| f.instructions_covered == 0).collect();
        let partial: Vec<&FunctionRow> = rep.functions.iter().filter(|f| f.instructions_covered > 0 && f.blocks_covered < f.blocks).collect();
        println!("  functions never entered: {}; partly covered: {}", never.len(), partial.len());
    }
}

/// Gap list for collaborators: one row per uncovered block.
pub fn write_gaps_csv(rep: &Report, path: &Path) -> Result<()> {
    let mut s = String::from("function,cpu_addr,prg_offset,instructions,instructions_covered,blocks,blocks_covered,uncovered_block\n");
    for f in &rep.functions {
        if f.blocks_covered == f.blocks {
            continue;
        }
        for b in &f.uncovered_blocks {
            s.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                f.name,
                f.cpu_addr.clone().unwrap_or_default(),
                f.offset,
                f.instructions,
                f.instructions_covered,
                f.blocks,
                f.blocks_covered,
                b
            ));
        }
    }
    std::fs::write(path, s)?;
    Ok(())
}
