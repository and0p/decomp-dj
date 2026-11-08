//! Queries over the local database and the symbol maps, shared by the `dj` query commands
//! and the MCP server (docs/mcp.md). Every result is plain data that serializes to JSON.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::analysis::{self, AnalyzedFunction, CodeEdge, FixedPrg, Function};
use crate::cache;
use crate::collection::{Collection, Status};
use crate::coverage::{self, Report};

/// A project opened for queries: the files, the database (read-only) and the symbol maps.
pub struct Project {
    pub col: Collection,
    pub conn: Connection,
    pub setup: String,
    pub rom: Option<Vec<u8>>,
    /// Every function in every symbol map, for names and containing-function lookups.
    pub funcs: Vec<Function>,
    /// PRG bytes (the ROM without its header), when the ROM is known.
    pub prg: Option<Vec<u8>>,
    /// Why there is no ROM, when one is set but can't be read.
    rom_problem: Option<String>,
    /// Where PRG sits in CPU space, when the ROM alone says so.
    fixed_prg: Option<FixedPrg>,
    report: Option<Report>,
}

impl Project {
    pub fn open(dir: &Path, rom: Option<&Path>) -> Result<Project> {
        let col = Collection::open(dir)?;
        let conn = cache::Cache::open_readonly(&col)?;
        let setup: String = conn
            .query_row("SELECT setup FROM contents ORDER BY played_at DESC LIMIT 1", [], |r| r.get(0))
            .context("the local database is empty; run `dj clean` first")?;
        let rom_path = match rom {
            Some(r) => Some(r.to_path_buf()),
            None => col.settings()?.rom,
        };
        let (rom, rom_problem) = match rom_path {
            None => (None, None),
            Some(p) => match std::fs::read(&p) {
                Ok(bytes) => (Some(bytes), None),
                Err(e) => (None, Some(format!("the ROM at {} can't be read: {e}", p.display()))),
            },
        };
        let ann = crate::annotations::Annotations::load(&col.root)?;
        let mut funcs: Vec<Function> = analysis::project_functions(&col, rom.as_deref(), &ann)?.into_iter().map(|(f, _)| f).collect();
        funcs.sort_by_key(|f| (f.region.clone(), f.offset));
        let prg = rom.as_ref().map(|rom| {
            let header = match col.config.symbols.first() {
                Some(s) => s.rom_header.unwrap_or_else(|| analysis::default_rom_header(&s.format, Some(rom))),
                None => analysis::default_rom_header("", Some(rom)),
            } as usize;
            rom[header.min(rom.len())..].to_vec()
        });
        let fixed_prg = rom.as_deref().and_then(FixedPrg::of_rom);
        Ok(Project { col, conn, setup, rom, funcs, prg, rom_problem, fixed_prg, report: None })
    }

    /// The coverage report, worked out once per opened project.
    pub fn report(&mut self) -> Result<&Report> {
        if self.report.is_none() {
            self.report = Some(coverage::build_report(&self.col, &self.conn, self.rom.as_deref(), Some(&self.setup))?);
        }
        Ok(self.report.as_ref().unwrap())
    }

    fn analyzed(&mut self) -> Result<&[AnalyzedFunction]> {
        let rep = self.report()?;
        if !rep.symbols {
            bail!("this needs a symbol map (config.toml [[symbols]]) and the ROM");
        }
        Ok(&self.report.as_ref().unwrap().detail.analyzed)
    }

    /// Whether the symbol maps describe any code in PRG.
    pub fn has_symbols(&mut self) -> Result<bool> {
        Ok(self.report()?.symbols)
    }

    /// The PRG bytes, or why they aren't available.
    fn need_prg(&self) -> Result<Vec<u8>> {
        self.prg.clone().ok_or_else(|| match &self.rom_problem {
            Some(p) => anyhow!("{p}"),
            None => anyhow!("this needs the ROM (pass --rom once, or set it in .local/local.toml)"),
        })
    }

    /// The PRG bytes and the flags of what ran, for working from coverage alone. That means
    /// reading the bytes as 6502, so only NES ROMs can do without symbols.
    fn need_sweep(&mut self) -> Result<(Vec<u8>, Vec<u8>)> {
        let prg = self.need_prg()?;
        if !self.rom.as_deref().is_some_and(|r| r.starts_with(b"NES\x1a")) {
            bail!("this needs a symbol map (config.toml [[symbols]]): without one it only works on NES ROMs, whose code it can read as 6502");
        }
        let flags = self.report()?.detail.prg.clone();
        Ok((prg, flags))
    }

    fn cpu_text(&self, offset: u64) -> Option<String> {
        self.fixed_prg.and_then(|m| m.cpu_of(offset)).map(|c| format!("${c:04X}"))
    }

    fn where_text(&self, offset: u64) -> String {
        self.cpu_text(offset).unwrap_or_else(|| format!("PRG:{offset:#x}"))
    }

    fn sweep_note(&self) -> String {
        let cpu = if self.fixed_prg.is_some() { "" } else { " PRG here is bankable, so CPU addresses and JSR/JMP targets are unknown: calls aren't followed." };
        format!("no symbol map: worked out by sweeping linearly from the instructions that ran, so uncovered bytes shown as code may be data. Add [[symbols]] to config.toml for real functions.{cpu}")
    }

    /// Resolves an address: `PRG:0x1F3A` (a region offset), `0x1F3A` (a PRG offset),
    /// `$C123` or `cpu:0xC123` (a CPU address, through the symbol maps), or a symbol
    /// name, optionally `name+0x10`.
    pub fn resolve(&self, what: &str) -> Result<Addr> {
        let what = what.trim();
        let cpu = what.strip_prefix('$').or_else(|| what.strip_prefix("cpu:")).or_else(|| what.strip_prefix("CPU:"));
        if let Some(c) = cpu {
            let c = parse_num_hex(c)?;
            let hits: Vec<&Function> = self.funcs.iter().filter(|f| f.cpu_addr.is_some_and(|b| c >= b && c < b + f.size.max(1))).collect();
            let Some(f) = hits.first() else {
                return match self.fixed_prg.and_then(|m| m.offset_of(c)) {
                    Some(off) => Ok(self.addr("PRG", off)),
                    None => Err(anyhow!("CPU address ${c:04X} is not inside any function in the symbol maps; give a ROM offset (PRG:0x1234) instead")),
                };
            };
            let offset = f.offset + (c - f.cpu_addr.unwrap());
            let mut a = self.addr(&f.region, offset);
            if hits.len() > 1 {
                a.note = Some(format!("${c:04X} is in {} banks; using {}. Pass a ROM offset to pick another.", hits.len(), a.symbol.clone().unwrap_or_default()));
            }
            return Ok(a);
        }
        if let Ok((region, offset)) = crate::player::parse_region_offset(what) {
            return Ok(self.addr(&region, offset));
        }
        if what.starts_with("0x") || what.starts_with("0X") {
            return Ok(self.addr("PRG", parse_num_hex(what)?));
        }
        let (name, plus) = match what.split_once('+') {
            Some((n, p)) => (n.trim(), parse_num(p.trim())?),
            None => (what, 0),
        };
        let f = self.funcs.iter().find(|f| f.name == name).ok_or_else(|| {
            let close: Vec<&str> = self.funcs.iter().filter(|f| f.name.to_lowercase().contains(&name.to_lowercase())).map(|f| f.name.as_str()).take(5).collect();
            if close.is_empty() {
                anyhow!("{what} is not an address (PRG:0x1234, 0x1234, $C123) or a symbol in the project's symbol maps")
            } else {
                anyhow!("no symbol {name}; similar: {}", close.join(", "))
            }
        })?;
        Ok(self.addr(&f.region.clone(), f.offset + plus))
    }

    /// Describes `region:offset` with its CPU address and containing symbol, when known.
    pub fn addr(&self, region: &str, offset: u64) -> Addr {
        let f = self.function_at(region, offset);
        Addr {
            region: region.to_string(),
            offset,
            text: format!("{region}:{offset:#x}"),
            cpu: f.and_then(|f| f.cpu_addr.map(|c| format!("${:04X}", c + (offset - f.offset)))).or_else(|| if region == "PRG" { self.cpu_text(offset) } else { None }),
            symbol: f.map(|f| if offset == f.offset { f.name.clone() } else { format!("{}+{:#x}", f.name, offset - f.offset) }),
            note: None,
        }
    }

    pub fn function_at(&self, region: &str, offset: u64) -> Option<&Function> {
        self.funcs.iter().filter(|f| f.region == region && offset >= f.offset && offset < f.offset + f.size.max(1)).min_by_key(|f| f.size)
    }

    fn short(&self, id: &str) -> String {
        id[..8.min(id.len())].to_string()
    }

    fn title(&self, id: &str) -> String {
        self.col.replays.get(id).map(|r| r.title().to_string()).unwrap_or_default()
    }

    fn nearest_snapshot(&self, keys: &std::collections::BTreeMap<String, (Option<String>, Status)>, id: &str, frame: u64) -> Result<Option<u64>> {
        let Some(k) = keys.get(id).and_then(|(k, _)| k.clone()) else { return Ok(None) };
        let n: Option<i64> = self.conn.query_row("SELECT max(frame) FROM frames WHERE content = ?1 AND frame <= ?2", rusqlite::params![k, frame as i64], |r| r.get(0))?;
        Ok(n.map(|n| n as u64))
    }

    /// Segments that run `region` `start..=end`, best first.
    pub fn hits(&self, region: &str, start: u64, end: u64, limit: usize, cursor: usize) -> Result<Page<SegmentHit>> {
        let hits = coverage::first_hits_in(&self.col, &self.conn, &self.setup, region, start, end)?;
        let keys = cache::current_keys(&self.conn, &self.col, &self.setup)?;
        let total = hits.len();
        let mut items = vec![];
        for h in hits.into_iter().skip(cursor).take(limit) {
            items.push(SegmentHit {
                segment: h.id.clone(),
                short: self.short(&h.id),
                title: self.title(&h.id),
                frame: h.frame,
                at: self.addr(region, h.offset).text,
                depth: h.depth,
                status: h.status.label().to_string(),
                counts: h.status != Status::Dirty && !self.col.is_draft(&h.id),
                nearest_snapshot: self.nearest_snapshot(&keys, &h.id, h.frame)?,
            });
        }
        Ok(Page::new(items, total, cursor))
    }

    /// `where_runs`: which segments first run this address, and at which frame.
    pub fn xref(&self, what: &str, limit: usize, cursor: usize) -> Result<Xref> {
        let address = self.resolve(what)?;
        let hits = self.hits(&address.region, address.offset, address.offset, limit, cursor)?;
        Ok(Xref { address, hits })
    }

    /// The headline numbers of the coverage report.
    pub fn summary(&mut self) -> Result<Summary> {
        self.report()?;
        let rep = self.report.as_ref().unwrap();
        let t = &rep.totals;
        let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { (a as f64 * 1000.0 / b as f64).round() / 10.0 };
        Ok(Summary {
            setup: rep.setup.clone(),
            symbols: rep.symbols,
            instructions: Ratio { covered: t.instructions_covered, total: t.instructions, percent: pct(t.instructions_covered, t.instructions) },
            blocks: Ratio { covered: t.blocks_covered, total: t.blocks, percent: pct(t.blocks_covered, t.blocks) },
            functions: Ratio { covered: t.functions_entered, total: t.functions, percent: pct(t.functions_entered, t.functions) },
            by_play_alone: Ratio { covered: t.played_only_instructions_covered, total: t.instructions, percent: pct(t.played_only_instructions_covered, t.instructions) },
            unreachable_instructions: t.unreachable_instructions,
            data_bytes: t.data_bytes,
            annotation_warnings: rep.annotation_warnings.iter().map(|w| w.text(&self.col)).collect(),
            executed_outside_symbols: t.executed_outside_symbols,
            segments: rep.replays.len(),
            segments_left_out: LeftOut { dirty: t.dirty_segments, unplayed: t.unplayed_segments },
            functions_never_entered: rep.functions.iter().filter(|f| f.instructions_covered == 0).count(),
            functions_partly_covered: rep.functions.iter().filter(|f| f.instructions_covered > 0 && f.blocks_covered < f.blocks).count(),
            top_segments: {
                let mut r: Vec<_> = rep.replays.iter().filter(|r| r.unique > 0).collect();
                r.sort_by_key(|r| std::cmp::Reverse(r.unique));
                r.iter().take(5).map(|r| format!("{} {} (+{} only it runs)", &r.id[..8], r.title, r.unique)).collect()
            },
        })
    }

    /// Marks what annotations.toml calls data or unreachable as taken, so it isn't a gap.
    fn mark_annotated(&mut self, exec: &mut [bool]) -> Result<()> {
        let d = &self.report()?.detail;
        for (a, b) in d.data.iter().chain(&d.unreachable) {
            for o in *a..=*b {
                if let Some(e) = exec.get_mut(o as usize) {
                    *e = true;
                }
            }
        }
        Ok(())
    }

    /// Uncovered PRG with no symbol map to divide it up: the runs of bytes no instruction that
    /// ran occupies, with how covered code leads into them. Data shows up here too.
    pub fn uncovered_ranges(&mut self, sort: GapSort, limit: usize, cursor: usize) -> Result<Page<UncoveredRange>> {
        let (prg, flags) = self.need_sweep()?;
        let edges = analysis::covered_edges_6502(&prg, &flags, self.fixed_prg);
        let mut exec = analysis::executed_bytes_6502(&prg, &flags);
        self.mark_annotated(&mut exec)?;
        let mut rows: Vec<UncoveredRange> = analysis::uncovered_ranges(&exec)
            .into_iter()
            .map(|(start, end)| {
                let into: Vec<&CodeEdge> = edges.iter().filter(|e| e.to >= start && e.to < end).collect();
                let best = into.iter().max_by_key(|e| analysis::sweep_len_6502(&prg, &flags, e.to));
                UncoveredRange {
                    at: format!("PRG:{start:#x}"),
                    end: format!("PRG:{:#x}", end - 1),
                    cpu: self.cpu_text(start),
                    bytes: end - start,
                    ways_in: into.len(),
                    best_way_in: best.map(|e| format!("PRG:{:#x}", e.from)),
                }
            })
            .collect();
        match sort {
            GapSort::Size => rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.at.cmp(&b.at))),
            GapSort::Nearness => rows.sort_by(|a, b| (b.ways_in > 0).cmp(&(a.ways_in > 0)).then(b.bytes.cmp(&a.bytes)).then(a.at.cmp(&b.at))),
        }
        let total = rows.len();
        let note = format!("no symbol map: these are PRG byte ranges that no instruction that ran occupies, and they include any data annotations.toml doesn't mark. ways_in counts covered branches and calls that lead into them (frontier lists them). Add [[symbols]] to config.toml for gaps by function.{}", if self.fixed_prg.is_some() { "" } else { " PRG here is bankable, so calls aren't followed." });
        Ok(Page::new(rows.into_iter().skip(cursor).take(limit).collect(), total, cursor).with_note(note))
    }

    /// Uncovered code, one row per function that has any, ranked by `sort`.
    pub fn gaps(&mut self, sort: GapSort, only: GapFilter, limit: usize, cursor: usize) -> Result<Page<Gap>> {
        let prg = self.need_prg()?;
        self.analyzed()?;
        let rep = self.report.as_ref().unwrap();
        let d = &rep.detail;
        let mut rows = vec![];
        for f in &d.analyzed {
            let covered = f.instructions.iter().filter(|i| d.hit(i.offset)).count();
            if covered == f.instructions.len() || f.instructions.is_empty() {
                continue;
            }
            let entered = covered > 0;
            match only {
                GapFilter::All => {}
                GapFilter::Never if entered => continue,
                GapFilter::Partial if !entered => continue,
                _ => {}
            }
            let succ = analysis::block_successors_6502(f, &prg);
            let uncovered: Vec<usize> = (0..f.blocks.len()).filter(|i| !d.hit(f.blocks[*i].offset)).collect();
            let near: HashSet<u64> = f.blocks.iter().zip(&succ).filter(|(b, _)| d.hit(b.offset)).flat_map(|(_, s)| s.iter().copied()).filter(|o| !d.hit(*o)).collect();
            let biggest = uncovered.iter().map(|i| &f.blocks[*i]).filter(|b| near.contains(&b.offset)).max_by_key(|b| b.instructions.len());
            rows.push(Gap {
                function: f.func.name.clone(),
                at: format!("PRG:{:#x}", f.func.offset),
                cpu: f.func.cpu_addr.map(|c| format!("${c:04X}")),
                instructions: f.instructions.len(),
                uncovered_instructions: f.instructions.len() - covered,
                blocks: f.blocks.len(),
                uncovered_blocks: uncovered.len(),
                entered,
                skipped_by_covered_branch: near.len(),
                best_frontier_block: biggest.map(|b| match b.cpu_addr {
                    Some(c) => format!("${c:04X}"),
                    None => format!("PRG:{:#x}", b.offset),
                }),
            });
        }
        match sort {
            GapSort::Size => rows.sort_by(|a, b| b.uncovered_instructions.cmp(&a.uncovered_instructions).then(a.at.cmp(&b.at))),
            GapSort::Nearness => rows.sort_by(|a, b| (b.skipped_by_covered_branch > 0).cmp(&(a.skipped_by_covered_branch > 0)).then(b.entered.cmp(&a.entered)).then(b.uncovered_instructions.cmp(&a.uncovered_instructions))),
        }
        let total = rows.len();
        Ok(Page::new(rows.into_iter().skip(cursor).take(limit).collect(), total, cursor))
    }

    /// One function in detail: its blocks, who enters it, and its covered callers.
    pub fn function(&mut self, what: &str, segments_limit: usize) -> Result<FunctionInfo> {
        let a = self.resolve(what)?;
        let prg = self.need_prg()?;
        if !self.has_symbols()? {
            return self.function_by_sweep(&a, segments_limit);
        }
        self.analyzed()?;
        let rep = self.report.as_ref().unwrap();
        let d = &rep.detail;
        let f = d
            .analyzed
            .iter()
            .find(|f| a.region == f.func.region && a.offset >= f.func.offset && a.offset < f.func.offset + f.func.size.max(1))
            .ok_or_else(|| anyhow!("{} is not inside a function the symbol maps describe", a.text))?;
        let firsts = self.first_frames(&f.func.region, f.func.offset, f.func.offset + f.func.size)?;
        let succ = analysis::block_successors_6502(f, &prg);
        let blocks = f
            .blocks
            .iter()
            .zip(&succ)
            .map(|(b, s)| BlockRow {
                at: format!("PRG:{:#x}", b.offset),
                cpu: b.cpu_addr.map(|c| format!("${c:04X}")),
                instructions: b.instructions.len(),
                covered: d.hit(b.offset),
                first_run: firsts.get(&b.offset).map(|(id, fr)| FirstRun { segment: id.clone(), short: self.short(id), title: self.title(id), frame: *fr }),
                next: s.iter().map(|o| f.blocks.iter().find(|b| b.offset == *o).and_then(|b| b.cpu_addr).map(|c| format!("${c:04X}")).unwrap_or_else(|| format!("PRG:{o:#x}"))).collect(),
            })
            .collect();
        let callers = self.callers(f, &prg);
        let info = FunctionInfo {
            note: None,
            name: f.func.name.clone(),
            at: format!("PRG:{:#x}", f.func.offset),
            cpu: f.func.cpu_addr.map(|c| format!("${c:04X}")),
            size: f.func.size,
            instructions: f.instructions.len(),
            instructions_covered: f.instructions.iter().filter(|i| d.hit(i.offset)).count(),
            blocks,
            callers,
            entered_by: Page { items: vec![], total: 0, next_cursor: None, note: None },
        };
        let entry = (info.at.clone(), f.func.region.clone(), f.func.offset);
        let mut info = info;
        info.entered_by = self.hits(&entry.1, entry.2, entry.2, segments_limit, 0)?;
        Ok(info)
    }

    /// `function` with no symbol map: the routine a linear sweep finds around the address.
    fn function_by_sweep(&mut self, a: &Addr, segments_limit: usize) -> Result<FunctionInfo> {
        let (prg, flags) = self.need_sweep()?;
        let size = flags.len().min(prg.len()) as u64;
        if a.region != "PRG" || a.offset >= size {
            bail!("{} is not in PRG ({size} bytes)", a.text);
        }
        const SWEEP_BYTES: u64 = 0x400;
        let (start, end) = analysis::sweep_bounds_6502(&prg, &flags, a.offset, SWEEP_BYTES);
        // Branches resolve through CPU addresses, so a ROM with none gets a stand-in; rows
        // only show it when it is real.
        let known = self.cpu_text(start).is_some();
        let cpu_addr = self.fixed_prg.and_then(|m| m.cpu_of(start)).unwrap_or(0x8000);
        let func = Function { name: format!("sweep_{start:04X}"), region: "PRG".into(), offset: start, size: end - start, cpu_addr: Some(cpu_addr) };
        let data = self.report()?.detail.data.clone();
        let analyzed = analysis::analyze_6502_with(std::slice::from_ref(&func), &prg, &data);
        let f = analyzed.first().ok_or_else(|| anyhow!("nothing to read at {}", a.text))?;
        let firsts = self.first_frames("PRG", start, end)?;
        let succ = analysis::block_successors_6502(f, &prg);
        let hit = |o: u64| flags.get(o as usize).is_some_and(|b| b & coverage::FLAG_OPCODE != 0);
        let show = |o: u64, c: Option<u64>| match c.filter(|_| known) {
            Some(c) => format!("${c:04X}"),
            None => format!("PRG:{o:#x}"),
        };
        let blocks = f
            .blocks
            .iter()
            .zip(&succ)
            .map(|(b, s)| BlockRow {
                at: format!("PRG:{:#x}", b.offset),
                cpu: b.cpu_addr.filter(|_| known).map(|c| format!("${c:04X}")),
                instructions: b.instructions.len(),
                covered: hit(b.offset),
                first_run: firsts.get(&b.offset).map(|(id, fr)| FirstRun { segment: id.clone(), short: self.short(id), title: self.title(id), frame: *fr }),
                next: s.iter().map(|o| show(*o, f.blocks.iter().find(|b| b.offset == *o).and_then(|b| b.cpu_addr))).collect(),
            })
            .collect();
        // A call or jump that ran, wherever it is, and lands on this routine's first byte.
        let mut callers = vec![];
        if let Some(map) = self.fixed_prg {
            for (off, _) in flags.iter().enumerate().filter(|(_, b)| **b & coverage::FLAG_OPCODE != 0) {
                let bytes = &prg[off.min(prg.len())..(off + 3).min(prg.len())];
                if let Some((t, call)) = analysis::call_target_6502(bytes) {
                    if map.offset_of(t) == Some(start) {
                        let off = off as u64;
                        callers.push(Caller { function: "(no symbols)".into(), at: format!("PRG:{off:#x}"), cpu: self.cpu_text(off), kind: if call { "JSR" } else { "JMP" }.into(), covered: true });
                    }
                }
            }
        }
        Ok(FunctionInfo {
            note: Some(self.sweep_note()),
            name: func.name.clone(),
            at: format!("PRG:{start:#x}"),
            cpu: self.cpu_text(start),
            size: end - start,
            instructions: f.instructions.len(),
            instructions_covered: f.instructions.iter().filter(|i| hit(i.offset)).count(),
            blocks,
            callers,
            entered_by: self.hits("PRG", start, start, segments_limit, 0)?,
        })
    }

    /// The first segment (best first) and frame to run each offset of `start..end`.
    fn first_frames(&self, region: &str, start: u64, end: u64) -> Result<HashMap<u64, (String, u64)>> {
        let keys = cache::current_keys(&self.conn, &self.col, &self.setup)?;
        let mut by_key: HashMap<String, (String, usize, bool)> = HashMap::new();
        for (id, (k, st)) in &keys {
            if let Some(k) = k {
                by_key.insert(k.clone(), (id.clone(), self.col.chain(id).map(|c| c.len()).unwrap_or(0), *st == Status::Dirty));
            }
        }
        let mut st = self.conn.prepare_cached("SELECT content, rom_offset, frame FROM first_hits WHERE region = ?1 AND rom_offset >= ?2 AND rom_offset < ?3")?;
        let rows = st.query_map(rusqlite::params![region, start as i64, end as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64)))?;
        let mut best: HashMap<u64, ((bool, usize, u64), String, u64)> = HashMap::new();
        for row in rows {
            let (content, off, frame) = row?;
            let Some((id, depth, dirty)) = by_key.get(&content) else { continue };
            let rank = (*dirty, *depth, frame);
            let e = best.entry(off).or_insert((rank, id.clone(), frame));
            if rank < e.0 {
                *e = (rank, id.clone(), frame);
            }
        }
        Ok(best.into_iter().map(|(o, (_, id, f))| (o, (id, f))).collect())
    }

    /// JSR and JMP sites anywhere in the analyzed code that target `f`'s entry.
    fn callers(&self, f: &AnalyzedFunction, prg: &[u8]) -> Vec<Caller> {
        let Some(entry) = f.func.cpu_addr else { return vec![] };
        let d = &self.report.as_ref().unwrap().detail;
        let mut out = vec![];
        for g in &d.analyzed {
            for i in &g.instructions {
                let bytes = &prg[(i.offset as usize).min(prg.len())..((i.offset + i.len as u64) as usize).min(prg.len())];
                if let Some((t, call)) = analysis::call_target_6502(bytes) {
                    if t == entry {
                        out.push(Caller {
                            function: g.func.name.clone(),
                            at: format!("PRG:{:#x}", i.offset),
                            cpu: g.func.cpu_addr.map(|c| format!("${:04X}", c + (i.offset - g.func.offset))),
                            kind: if call { "JSR" } else { "JMP" }.into(),
                            covered: d.hit(i.offset),
                        });
                    }
                }
            }
        }
        out.sort_by_key(|c| !c.covered);
        out
    }

    /// Where to start exploring toward uncovered code. For an address in an uncovered block,
    /// the covered blocks that branch or fall into it; for a function never entered, its
    /// covered callers; with no address, the largest such edges across the ROM.
    pub fn frontier(&mut self, what: Option<&str>, limit: usize, cursor: usize) -> Result<Page<FrontierEdge>> {
        let prg = self.need_prg()?;
        let target = what.map(|w| self.resolve(w)).transpose()?;
        if !self.has_symbols()? {
            return self.frontier_by_sweep(target.as_ref(), limit, cursor);
        }
        self.analyzed()?;
        let rep = self.report.as_ref().unwrap();
        let d = &rep.detail;
        // (from function, from block, branch instruction offset, target block offset, target size, target function)
        let mut edges: Vec<(usize, usize, u64, u64, usize)> = vec![];
        let mut callers_of: Vec<usize> = vec![];
        for (fi, f) in d.analyzed.iter().enumerate() {
            if let Some(t) = &target {
                let inside = t.region == f.func.region && t.offset >= f.func.offset && t.offset < f.func.offset + f.func.size.max(1);
                if !inside {
                    continue;
                }
                if !f.instructions.iter().any(|i| d.hit(i.offset)) {
                    callers_of.push(fi);
                    continue;
                }
            }
            let succ = analysis::block_successors_6502(f, &prg);
            for (bi, (b, s)) in f.blocks.iter().zip(&succ).enumerate() {
                if !d.hit(b.offset) {
                    continue;
                }
                for o in s {
                    if d.hit(*o) {
                        continue;
                    }
                    if let Some(t) = &target {
                        // Only edges into the block that holds the address, or any block of the function when given its entry.
                        let tb = f.blocks.iter().rposition(|b| b.offset <= t.offset).unwrap_or(0);
                        if t.offset != f.func.offset && f.blocks[tb].offset != *o {
                            continue;
                        }
                    }
                    // Size: the uncovered blocks reachable from the target inside the function.
                    let size = uncovered_reach(f, &succ, *o, d);
                    edges.push((fi, bi, *b.instructions.last().unwrap_or(&b.offset), *o, size));
                }
            }
        }
        let mut out = vec![];
        for (fi, bi, branch, to, size) in &edges {
            let f = &d.analyzed[*fi];
            let b = &f.blocks[*bi];
            let tb = f.blocks.iter().find(|x| x.offset == *to);
            let cpu = |o: u64| f.func.cpu_addr.map(|c| format!("${:04X}", c + (o - f.func.offset)));
            out.push(FrontierEdge {
                kind: "branch".into(),
                function: f.func.name.clone(),
                uncovered: tb.and_then(|x| x.cpu_addr).map(|c| format!("${c:04X}")).unwrap_or_else(|| format!("PRG:{to:#x}")),
                uncovered_instructions: *size,
                from_block: b.cpu_addr.map(|c| format!("${c:04X}")).unwrap_or_else(|| format!("PRG:{:#x}", b.offset)),
                branch: cpu(*branch).unwrap_or_else(|| format!("PRG:{branch:#x}")),
                branch_at: format!("PRG:{branch:#x}"),
                reached_by: vec![],
            });
        }
        for fi in callers_of {
            let f = &d.analyzed[fi];
            for c in self.callers(f, &prg).into_iter().filter(|c| c.covered) {
                out.push(FrontierEdge {
                    kind: c.kind.to_lowercase(),
                    function: f.func.name.clone(),
                    uncovered: f.func.cpu_addr.map(|c| format!("${c:04X}")).unwrap_or_else(|| format!("PRG:{:#x}", f.func.offset)),
                    uncovered_instructions: f.instructions.len(),
                    from_block: c.function.clone(),
                    branch: c.cpu.clone().unwrap_or_else(|| c.at.clone()),
                    branch_at: c.at.clone(),
                    reached_by: vec![],
                });
            }
        }
        out.sort_by(|a, b| b.uncovered_instructions.cmp(&a.uncovered_instructions).then(a.branch_at.cmp(&b.branch_at)));
        let total = out.len();
        let mut page: Vec<FrontierEdge> = out.into_iter().skip(cursor).take(limit).collect();
        for e in &mut page {
            let (region, off) = crate::player::parse_region_offset(&e.branch_at)?;
            e.reached_by = self.hits(&region, off, off, 3, 0)?.items;
        }
        if total == 0 && target.is_some() {
            bail!("no covered code leads directly into it: either it is covered already, or nothing that runs branches or calls there (an indirect jump, an interrupt or a table may reach it)");
        }
        Ok(Page::new(page, total, cursor))
    }

    /// `frontier` with no symbol map: edges out of the instructions that ran. For an address,
    /// those into the uncovered range that holds it.
    fn frontier_by_sweep(&mut self, target: Option<&Addr>, limit: usize, cursor: usize) -> Result<Page<FrontierEdge>> {
        let (prg, flags) = self.need_sweep()?;
        let mut exec = analysis::executed_bytes_6502(&prg, &flags);
        self.mark_annotated(&mut exec)?;
        let mut edges = analysis::covered_edges_6502(&prg, &flags, self.fixed_prg);
        // No way into data or unreachable code counts (they are marked taken in `exec` now).
        edges.retain(|e| !exec.get(e.to as usize).copied().unwrap_or(false) || flags.get(e.to as usize).is_some_and(|b| b & coverage::FLAG_OPCODE != 0));
        if let Some(t) = target {
            let range = analysis::uncovered_ranges(&exec).into_iter().find(|(s, e)| t.region == "PRG" && t.offset >= *s && t.offset < *e);
            edges.retain(|e| range.is_some_and(|(s, end)| e.to >= s && e.to < end));
            if edges.is_empty() {
                bail!("no covered code leads directly into it: either it is covered already, or nothing that runs branches or calls there (an indirect jump, an interrupt or a table may reach it)");
            }
        }
        let mut rows: Vec<(usize, CodeEdge)> = edges.into_iter().map(|e| (analysis::sweep_len_6502(&prg, &flags, e.to), e)).collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.from.cmp(&b.1.from)).then(a.1.to.cmp(&b.1.to)));
        let total = rows.len();
        let mut page = vec![];
        for (size, e) in rows.into_iter().skip(cursor).take(limit) {
            page.push(FrontierEdge {
                kind: e.kind.to_string(),
                function: "(no symbols)".into(),
                uncovered: self.where_text(e.to),
                uncovered_instructions: size,
                from_block: self.where_text(e.from),
                branch: self.where_text(e.from),
                branch_at: format!("PRG:{:#x}", e.from),
                reached_by: self.hits("PRG", e.from, e.from, 3, 0)?.items,
            });
        }
        Ok(Page::new(page, total, cursor).with_note(self.sweep_note()))
    }
}

fn uncovered_reach(f: &AnalyzedFunction, succ: &[Vec<u64>], start: u64, d: &coverage::Detail) -> usize {
    let mut seen = HashSet::new();
    let mut stack = vec![start];
    let mut n = 0;
    while let Some(o) = stack.pop() {
        if !seen.insert(o) || d.hit(o) {
            continue;
        }
        if let Some(i) = f.blocks.iter().position(|b| b.offset == o) {
            n += f.blocks[i].instructions.len();
            stack.extend(succ[i].iter().copied());
        }
    }
    n
}

fn parse_num_hex(s: &str) -> Result<u64> {
    let s = s.trim();
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(h, 16).map_err(|_| anyhow!("not a hex number: {s}"))
}

fn parse_num(s: &str) -> Result<u64> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).map_err(|_| anyhow!("not a number: {s}")),
        None => s.parse().map_err(|_| anyhow!("not a number: {s}")),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum GapSort {
    Size,
    Nearness,
}

impl std::str::FromStr for GapSort {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "size" => Ok(GapSort::Size),
            "nearness" | "near" => Ok(GapSort::Nearness),
            _ => bail!("sort is size or nearness"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum GapFilter {
    All,
    Never,
    Partial,
}

impl std::str::FromStr for GapFilter {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "all" => Ok(GapFilter::All),
            "never" | "never-entered" => Ok(GapFilter::Never),
            "partial" => Ok(GapFilter::Partial),
            _ => bail!("only is all, never or partial"),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Addr {
    pub region: String,
    pub offset: u64,
    /// `REGION:0xOFFSET`
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    /// Pass as `cursor` for the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<usize>,
    /// How to read these rows, when they aren't what they usually are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl<T> Page<T> {
    fn new(items: Vec<T>, total: usize, cursor: usize) -> Self {
        let next = cursor + items.len();
        Page { items, total, next_cursor: (next < total).then_some(next), note: None }
    }

    fn with_note(mut self, note: String) -> Self {
        self.note = Some(note);
        self
    }
}

#[derive(Serialize)]
pub struct SegmentHit {
    pub segment: String,
    pub short: String,
    pub title: String,
    /// First frame (inside the segment) that runs it.
    pub frame: u64,
    /// The exact offset that ran first (for a range).
    pub at: String,
    pub depth: usize,
    pub status: String,
    /// Dirty segments don't count toward coverage.
    pub counts: bool,
    /// Nearest snapshot at or before `frame` in the local database.
    pub nearest_snapshot: Option<u64>,
}

#[derive(Serialize)]
pub struct Xref {
    pub address: Addr,
    pub hits: Page<SegmentHit>,
}

#[derive(Serialize)]
pub struct Ratio {
    pub covered: usize,
    pub total: usize,
    pub percent: f64,
}

#[derive(Serialize)]
pub struct LeftOut {
    pub dirty: usize,
    pub unplayed: usize,
}

#[derive(Serialize)]
pub struct Summary {
    pub setup: String,
    pub symbols: bool,
    pub instructions: Ratio,
    pub blocks: Ratio,
    pub functions: Ratio,
    pub by_play_alone: Ratio,
    pub unreachable_instructions: usize,
    /// PRG bytes annotations.toml marks as data.
    pub data_bytes: usize,
    /// Code that ran where annotations.toml says none can.
    pub annotation_warnings: Vec<String>,
    pub executed_outside_symbols: usize,
    pub segments: usize,
    pub segments_left_out: LeftOut,
    pub functions_never_entered: usize,
    pub functions_partly_covered: usize,
    pub top_segments: Vec<String>,
}

#[derive(Serialize)]
pub struct Gap {
    pub function: String,
    pub at: String,
    pub cpu: Option<String>,
    pub instructions: usize,
    pub uncovered_instructions: usize,
    pub blocks: usize,
    pub uncovered_blocks: usize,
    /// Whether any of it runs.
    pub entered: bool,
    /// Uncovered blocks that covered code branches or falls into.
    pub skipped_by_covered_branch: usize,
    /// The largest of those, a good argument for `frontier`.
    pub best_frontier_block: Option<String>,
}

/// A run of PRG bytes no instruction that ran occupies, from `uncovered_ranges`.
#[derive(Serialize)]
pub struct UncoveredRange {
    pub at: String,
    /// Its last byte.
    pub end: String,
    pub cpu: Option<String>,
    pub bytes: u64,
    /// Covered branches and calls that lead into it.
    pub ways_in: usize,
    /// The instruction of those that opens up the most, a good argument for `where_runs`.
    pub best_way_in: Option<String>,
}

#[derive(Serialize)]
pub struct FirstRun {
    pub segment: String,
    pub short: String,
    pub title: String,
    pub frame: u64,
}

#[derive(Serialize)]
pub struct BlockRow {
    pub at: String,
    pub cpu: Option<String>,
    pub instructions: usize,
    pub covered: bool,
    pub first_run: Option<FirstRun>,
    /// Blocks control can go to next, inside the function.
    pub next: Vec<String>,
}

#[derive(Serialize)]
pub struct Caller {
    pub function: String,
    pub at: String,
    pub cpu: Option<String>,
    pub kind: String,
    pub covered: bool,
}

#[derive(Serialize)]
pub struct FunctionInfo {
    /// How to read this, when it didn't come from a symbol map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub name: String,
    pub at: String,
    pub cpu: Option<String>,
    pub size: u64,
    pub instructions: usize,
    pub instructions_covered: usize,
    pub blocks: Vec<BlockRow>,
    pub callers: Vec<Caller>,
    pub entered_by: Page<SegmentHit>,
}

#[derive(Serialize)]
pub struct FrontierEdge {
    /// "branch" (a covered block that can branch or fall into uncovered code), or "jsr"/"jmp"
    /// (a covered call to a function that never runs).
    pub kind: String,
    pub function: String,
    pub uncovered: String,
    /// Uncovered instructions reachable from there inside the function.
    pub uncovered_instructions: usize,
    pub from_block: String,
    /// The branch or call instruction that decides it.
    pub branch: String,
    pub branch_at: String,
    /// Segments and frames that run that instruction: where to open a session.
    pub reached_by: Vec<SegmentHit>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{Cache, CacheSettings, ContentRow, Recording};
    use crate::coverage::FLAG_OPCODE;

    /// A hydrated project with one segment and no symbol map, whose ROM is 32 KB of NROM:
    /// $8000 LDA #1; BNE $8007; INC $10; RTS; $8007 DEC $11; RTS. Only the LDA and BNE ran.
    fn project(name: &str, write_rom: bool) -> (std::path::PathBuf, Project) {
        project_with(name, write_rom, 2)
    }

    fn project_with(name: &str, write_rom: bool, prg_banks: u8) -> (std::path::PathBuf, Project) {
        project_full(name, write_rom, prg_banks, None, None)
    }

    /// `project`, with an annotations.toml and a CSV symbol map.
    fn project_full(name: &str, write_rom: bool, prg_banks: u8, annotations: Option<&str>, symbols: Option<&str>) -> (std::path::PathBuf, Project) {
        let dir = std::env::temp_dir().join(format!("dj-query-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("segments")).unwrap();
        std::fs::create_dir_all(dir.join(".local")).unwrap();
        std::fs::write(dir.join("config.toml"), "format = 1\nname = \"t\"\n[rom]\nsha256 = \"\"\nfile = \"t.nes\"\n[core]\nname = \"c\"\nbuild = \"c\"\nsha256 = \"\"\n[[ports]]\nport = 1\ndevice = \"JOYPAD\"\n[hash]\nalgorithm = \"blake3\"\nregions = [\"RAM\"]\n").unwrap();
        if let Some(csv) = symbols {
            std::fs::write(dir.join("symbols.csv"), csv).unwrap();
            let mut c = std::fs::OpenOptions::new().append(true).open(dir.join("config.toml")).unwrap();
            std::io::Write::write_all(&mut c, b"[[symbols]]\nfile = \"symbols.csv\"\nformat = \"csv\"\n").unwrap();
        }
        if let Some(a) = annotations {
            std::fs::write(dir.join("annotations.toml"), a).unwrap();
        }
        let id = "11111111-1111-1111-1111-111111111111";
        std::fs::write(dir.join("segments").join(format!("{id}.segment")), format!("dj-replay 1\nid: {id}\nparent: none\ntitle: s\nframes: 1\nstart-hash: a\nend-hash: b\n---\n")).unwrap();
        let rom_path = dir.join("t.nes");
        if write_rom {
            let mut rom = b"NES\x1a".to_vec();
            rom.push(prg_banks);
            rom.resize(16, 0);
            rom.extend([0xA9, 0x01, 0xD0, 0x03, 0xE6, 0x10, 0x60, 0xC6, 0x11, 0x60]);
            rom.resize(16 + 0x8000, 0xFF);
            std::fs::write(&rom_path, rom).unwrap();
        }
        std::fs::write(dir.join(".local").join("local.toml"), format!("rom = {:?}\n", rom_path.display().to_string())).unwrap();
        let col = Collection::open(&dir).unwrap();
        let body = cache::body_hash(&col.replays[id], &crate::player::devices_at_start(&col, id).unwrap()).unwrap();
        let key = cache::content_key("setup", "a", &body);
        let mut flags = vec![0u8; 0x8000];
        flags[0] = FLAG_OPCODE;
        flags[2] = FLAG_OPCODE;
        let mut c = Cache::open(&col, "setup", &[], CacheSettings::resolve(None, None).unwrap()).unwrap();
        let row = ContentRow { key, segment_id: id.into(), start_hash: "a".into(), end_hash: "b".into(), body, frames: 1, ticks: None };
        let rec = Recording { coverage: vec![("PRG".into(), flags)], first_hits: vec![("PRG".into(), 0, 0), ("PRG".into(), 2, 0)], ..Default::default() };
        c.record(&row, &rec).unwrap();
        drop(c);
        let p = Project::open(&dir, None).unwrap();
        (dir, p)
    }

    #[test]
    fn the_rom_loads_without_symbols() {
        let (dir, mut p) = project("rom", true);
        assert_eq!(p.prg.as_ref().map(|b| (b.len(), b[0])), Some((0x8000, 0xA9)));
        assert!(!p.has_symbols().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rom_that_cant_be_read_says_so() {
        let (dir, mut p) = project("norom", false);
        let e = p.uncovered_ranges(GapSort::Size, 10, 0).err().unwrap().to_string();
        assert!(e.contains("can't be read") && e.contains("t.nes"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gaps_without_symbols_are_ranges() {
        let (dir, mut p) = project("gaps", true);
        let g = p.uncovered_ranges(GapSort::Nearness, 10, 0).unwrap();
        assert_eq!(g.total, 1);
        let r = &g.items[0];
        assert_eq!((r.at.as_str(), r.end.as_str(), r.cpu.as_deref(), r.bytes, r.ways_in), ("PRG:0x4", "PRG:0x7fff", Some("$8004"), 0x7FFC, 2));
        assert_eq!(r.best_way_in.as_deref(), Some("PRG:0x2"));
        assert!(g.note.unwrap().contains("no symbol map"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn function_without_symbols_sweeps() {
        let (dir, mut p) = project("func", true);
        for addr in ["PRG:0x2", "$8002", "0x3"] {
            let f = p.function(addr, 5).unwrap();
            assert_eq!((f.at.as_str(), f.cpu.as_deref(), f.size, f.instructions, f.instructions_covered), ("PRG:0x0", Some("$8000"), 10, 6, 2), "{addr}");
            assert!(f.blocks[0].covered && f.blocks[0].first_run.is_some());
            assert_eq!(f.blocks[0].next, vec!["$8004", "$8007"]);
            assert!(!f.blocks[1].covered);
            assert_eq!(f.entered_by.total, 1);
            assert!(f.note.is_some());
        }
        assert!(p.function("PRG:0x8000", 5).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bankable_rom_has_no_cpu_addresses() {
        let (dir, mut p) = project_with("bank", true, 4);
        assert!(p.resolve("$8000").is_err());
        let f = p.function("PRG:0x2", 5).unwrap();
        assert_eq!(f.cpu, None);
        assert_eq!(f.blocks[0].cpu, None);
        assert_eq!(f.blocks[0].next, vec!["PRG:0x4", "PRG:0x7"]);
        assert!(f.note.unwrap().contains("bankable"));
        let e = p.frontier(None, 10, 0).unwrap();
        assert_eq!(e.items[0].branch, "PRG:0x2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn annotations_leave_data_out_without_symbols() {
        let ann = "[[unreachable]]\nrange = \"0x7-0x9\"\nwhy = \"dead\"\n[[data]]\nrange = \"0x100-0x7FFF\"\nwhy = \"tables\"\n";
        let (dir, mut p) = project_full("ann", true, 2, Some(ann), None);
        let s = p.summary().unwrap();
        assert_eq!((s.instructions.covered, s.instructions.total, s.unreachable_instructions, s.data_bytes), (2, 0x100 - 3, 3, 0x7F00));
        assert!(s.annotation_warnings.is_empty());
        // Neither is a gap, nor a way into one.
        let g = p.uncovered_ranges(GapSort::Size, 10, 0).unwrap();
        let ranges: Vec<(&str, &str)> = g.items.iter().map(|r| (r.at.as_str(), r.end.as_str())).collect();
        assert_eq!(ranges, vec![("PRG:0xa", "PRG:0xff"), ("PRG:0x4", "PRG:0x6")]);
        let f = p.frontier(None, 10, 0).unwrap();
        assert_eq!(f.items.iter().map(|e| e.uncovered.as_str()).collect::<Vec<_>>(), vec!["$8004"]);
        let d = &p.report().unwrap().detail;
        assert_eq!(d.annotation_at(0x8).map(|e| e.why.as_str()), Some("dead"));
        assert!(d.is_data(0x100) && !d.is_data(0xFF));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn annotations_warn_when_code_runs_there() {
        let ann = "[[data]]\nrange = \"0x1000\"\n[[unreachable]]\nrange = \"PRG:0x2-0x3\"\n";
        let (dir, mut p) = project_full("warn", true, 2, Some(ann), None);
        let w = p.report().unwrap().annotation_warnings.clone();
        assert_eq!(w.len(), 1);
        let w = &w[0];
        assert_eq!((w.entry.as_str(), w.range.as_str(), w.at.as_str()), ("unreachable entry 1", "PRG:0x0002-0x0003", "PRG:0x0002"));
        assert_eq!(w.segments, vec!["11111111-1111-1111-1111-111111111111"]);
        assert_eq!(w.text(&p.col), "annotations.toml unreachable entry 1 (PRG:0x0002-0x0003) ran as code at PRG:0x0002 in \"s\"");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn annotations_with_symbols() {
        // f covers the whole routine; bytes 4-6 (INC $10; RTS) are declared data, and a
        // function is added at 7 (DEC $11; RTS), which cuts f short.
        let ann = "[[data]]\nrange = \"0x4-0x6\"\n[[function]]\nat = \"0x7\"\nname = \"tail\"\nwhy = \"the branch target\"\n";
        let (dir, mut p) = project_full("sym", true, 2, Some(ann), Some("name,region,offset,size,cpu\nf,PRG,0,A,8000\n"));
        let r = p.report().unwrap();
        assert!(r.symbols);
        let rows: Vec<(&str, &str, usize, usize)> = r.functions.iter().map(|f| (f.name.as_str(), f.offset.as_str(), f.instructions, f.instructions_covered)).collect();
        assert_eq!(rows, vec![("f", "0x0", 2, 2), ("tail", "0x7", 2, 0)]);
        assert_eq!((r.totals.instructions, r.totals.data_bytes), (4, 3));
        assert_eq!(p.resolve("tail").unwrap().offset, 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frontier_without_symbols_lists_edges() {
        let (dir, mut p) = project("front", true);
        let all = p.frontier(None, 10, 0).unwrap();
        assert_eq!(all.total, 2);
        assert_eq!(all.items[0].branch, "$8002");
        assert_eq!(all.items[0].branch_at, "PRG:0x2");
        assert_eq!(all.items[0].kind, "branch");
        assert_eq!(all.items[0].uncovered, "$8004");
        assert_eq!(all.items[0].reached_by.len(), 1);
        assert_eq!(p.frontier(Some("$8007"), 10, 0).unwrap().total, 2);
        // Covered already.
        assert!(p.frontier(Some("PRG:0x0"), 10, 0).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
