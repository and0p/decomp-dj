mod mcp;
mod queries;
use decomp_dj::{cache, coverage, libretro};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use decomp_dj::collection::{sha256_file, Collection, Config, CoreConfig, HashConfig, LocalConfig, PortConfig, RomConfig, Status, SymbolsConfig};
use decomp_dj::format::Replay;
use decomp_dj::hydrate::Hydrator;
use decomp_dj::player::{PlayOptions, ReplayResult, Session};

#[derive(Parser)]
#[command(name = "dj", version, about = "Play libretro input-replay trees, check state hashes, and measure ROM code coverage")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
struct Local {
    /// ROM file (default: the one saved in .local/local.toml)
    #[arg(long, global = true)]
    rom: Option<PathBuf>,
    /// libretro core library (default: the one saved in .local/local.toml)
    #[arg(long, global = true)]
    core: Option<PathBuf>,
    /// Run without the local database (.local/cache.db), playing everything from power-on
    #[arg(long, global = true)]
    no_db: bool,
}

#[derive(clap::Args, Clone)]
struct QueryArgs {
    /// Print JSON instead of text
    #[arg(long)]
    json: bool,
    /// Rows to show
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Skip this many rows (the `next_cursor` of a previous page)
    #[arg(long, default_value_t = 0)]
    cursor: usize,
    /// ROM file (default: the one saved in .local/local.toml)
    #[arg(long)]
    rom: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a collection pinned to a ROM and a core, with every core option recorded
    Init {
        dir: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        rom: PathBuf,
        #[arg(long)]
        core: PathBuf,
        /// Free-form id of the exact core build (default: the core's own name and version)
        #[arg(long)]
        core_build: Option<String>,
        /// Core option overrides, KEY=VALUE
        #[arg(long = "option")]
        options: Vec<String>,
        /// Symbol map to copy into symbols/ (cc65 .dbg, splat symbol_addrs.txt, Mesen .mlb, GNU ld .map, ELF or CSV)
        #[arg(long)]
        symbols: Option<PathBuf>,
    },
    /// Create an empty segment
    New {
        dir: PathBuf,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value_t = 60)]
        frames: u64,
    },
    /// Show the segment tree with clean and dirty marks (no emulation)
    Status { dir: PathBuf },
    /// Play one segment (and its ancestors), check hashes, and save screenshots
    Play {
        dir: PathBuf,
        id: String,
        #[command(flatten)]
        local: Local,
        /// Save the last frame (and any [SCREENSHOT] frames) as PNGs here
        #[arg(long)]
        screenshots: Option<PathBuf>,
        /// Also capture these frames (relative to the segment)
        #[arg(long, value_delimiter = ',')]
        shot: Vec<u64>,
        /// Play from the root segment instead of a stored ancestor state
        #[arg(long)]
        no_cache: bool,
        /// Write the hashes this run produced into the replay files
        #[arg(long)]
        accept: bool,
    },
    /// Play every segment, check hashes, and record coverage
    Run {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
        /// Play each chain from its trunk instead of reusing parent states (determinism check)
        #[arg(long)]
        from_scratch: bool,
        /// Write the hashes this run produced into the replay files
        #[arg(long)]
        accept: bool,
        /// Save each segment's last frame to .local/screenshots/
        #[arg(long)]
        screenshots: bool,
    },
    /// Fill the local database without recording hashes: play the segments whose current
    /// content isn't stored yet (dj clean does this too, and records hashes)
    Hydrate {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
        /// Only this segment (id or unique prefix)
        segment: Option<String>,
        /// With a segment: also every segment below it
        #[arg(long)]
        children: bool,
        /// Play segments again even if their content is stored
        #[arg(long)]
        force: bool,
    },
    /// Play, top down, the segments that aren't clean or whose content isn't in the local
    /// database, and write the hashes they reach into their files (as Clean all segments does
    /// in dj-gui, which leaves them unsaved instead). A changed ending is listed, not written,
    /// and the command fails, unless --accept
    Clean {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
        /// Only this segment and every segment below it (id or unique prefix)
        segment: Option<String>,
        /// Also write changed endings (and go on below them)
        #[arg(long)]
        accept: bool,
        /// Replay every segment, even those whose content is stored
        #[arg(long)]
        all: bool,
    },
    /// Which segments first run an address, at which frame, nearest snapshot first
    #[command(alias = "where")]
    Xref {
        dir: PathBuf,
        /// PRG:0x1F3A, 0x1F3A (a PRG offset), $C123 (a CPU address) or a symbol name (name+0x10 works)
        what: String,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Coverage headline numbers
    Summary {
        dir: PathBuf,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Functions with uncovered code, largest or nearest first
    Gaps {
        dir: PathBuf,
        /// size (most uncovered instructions) or nearness (skipped by a covered branch first)
        #[arg(long, default_value = "size")]
        sort: decomp_dj::query::GapSort,
        /// all, never (functions never entered) or partial
        #[arg(long, default_value = "all")]
        only: decomp_dj::query::GapFilter,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// One function: its blocks, which run, which segments enter it, its callers
    Function {
        dir: PathBuf,
        what: String,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Covered branches and calls that lead into uncovered code, with the segments that reach them
    Frontier {
        dir: PathBuf,
        /// An address in uncovered code, or a function; leave out for the whole ROM
        what: Option<String>,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Serve the project to an agent over MCP (stdio); see docs/mcp.md
    Mcp {
        dir: PathBuf,
        /// What the agent may do: query, draft (the default) or save
        #[arg(long)]
        allow: Option<mcp::tools::Allow>,
    },
    /// Internal: one core driven by `dj mcp` over a pipe
    #[command(hide = true)]
    Worker { dir: PathBuf },
    /// Write the core state at a frame of a segment, to start a trace from
    Snapshot {
        dir: PathBuf,
        segment: String,
        frame: u64,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        local: Local,
    },
    /// Run an SQL query on the local database
    Sql { dir: PathBuf, query: String },
    /// Local database size, or clean it up
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
    /// Report coverage of the current segments
    Report {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
        /// Write the full report as JSON
        #[arg(long)]
        json: Option<PathBuf>,
        /// Write a CSV of uncovered blocks for collaborators
        #[arg(long)]
        gaps: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CacheCmd {
    /// What the local database holds
    Stats { dir: PathBuf },
    /// Drop content no current segment uses, and pages no snapshot uses. Refuses while a
    /// dj-gui has the project open (its unsaved edits' snapshots would go), unless --force
    Gc {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
        /// Run even while a dj-gui has the project open
        #[arg(long)]
        force: bool,
    },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(2);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { dir, name, rom, core, core_build, options, symbols } => init(&dir, &name, &rom, &core, core_build, &options, symbols.as_deref()),
        Cmd::New { dir, parent, title, frames } => new_replay(&dir, parent, &title, frames),
        Cmd::Status { dir } => status(&dir),
        Cmd::Play { dir, id, local, screenshots, shot, no_cache, accept } => play(&dir, &id, &local, screenshots, shot, no_cache, accept),
        Cmd::Run { dir, local, from_scratch, accept, screenshots } => run(&dir, &local, from_scratch, accept, screenshots),
        Cmd::Report { dir, local, json, gaps } => report(&dir, &local, json, gaps),
        Cmd::Hydrate { dir, local, segment, children, force } => hydrate(&dir, &local, segment, children, force),
        Cmd::Clean { dir, local, segment, accept, all } => clean(&dir, &local, segment, accept, all),
        Cmd::Xref { dir, what, query } => queries::xref(&dir, &what, &query),
        Cmd::Summary { dir, query } => queries::summary(&dir, &query),
        Cmd::Gaps { dir, sort, only, query } => queries::gaps(&dir, sort, only, &query),
        Cmd::Function { dir, what, query } => queries::function(&dir, &what, &query),
        Cmd::Frontier { dir, what, query } => queries::frontier(&dir, what.as_deref(), &query),
        Cmd::Snapshot { dir, segment, frame, out, local } => snapshot_cmd(&dir, &segment, frame, &out, &local),
        Cmd::Sql { dir, query } => sql(&dir, &query),
        Cmd::Worker { dir } => mcp::worker::run(dir),
        Cmd::Mcp { dir, allow } => mcp::serve(dir, mcp::tools::Options { allow }),
        Cmd::Cache { cmd: CacheCmd::Stats { dir } } => cache_stats(&dir),
        Cmd::Cache { cmd: CacheCmd::Gc { dir, local, force } } => cache_gc(&dir, &local, force),
    }
}

fn resolve_local(col: &Collection, local: &Local) -> Result<(PathBuf, PathBuf)> {
    let saved = col.settings()?;
    let rom = local.rom.clone().or(saved.rom).ok_or_else(|| anyhow!("no ROM given; pass --rom (it is then remembered)"))?;
    let core = local.core.clone().or(saved.core).ok_or_else(|| anyhow!("no core given; pass --core (it is then remembered)"))?;
    if local.rom.is_some() || local.core.is_some() {
        let mut saved = col.load_local()?;
        saved.rom = Some(std::fs::canonicalize(&rom)?);
        saved.core = Some(std::fs::canonicalize(&core)?);
        col.save_local(&saved)?;
    }
    Ok((rom, core))
}

fn open_session(col: &Collection, local: &Local) -> Result<Session> {
    let (rom, core) = resolve_local(col, local)?;
    let use_db = !local.no_db && col.settings()?.database.unwrap_or(true);
    Session::open_with(col, &rom, &core, use_db)
}

fn init(dir: &Path, name: &str, rom: &Path, core_path: &Path, core_build: Option<String>, options: &[String], symbols: Option<&Path>) -> Result<()> {
    if dir.join("config.toml").exists() || dir.join("collection.toml").exists() {
        bail!("{} already has a project", dir.display());
    }
    let mut overrides = BTreeMap::new();
    for o in options {
        let (k, v) = o.split_once('=').ok_or_else(|| anyhow!("--option takes KEY=VALUE"))?;
        overrides.insert(k.to_string(), v.to_string());
    }
    std::fs::create_dir_all(dir.join("segments"))?;
    std::fs::create_dir_all(dir.join("blobs"))?;
    let work = dir.join(".local");
    let (lib_name, lib_version, effective, regions) = {
        let mut core = libretro::Core::load(core_path, &work, &overrides)?;
        core.load_game(rom)?;
        let mut regions = vec![];
        for r in ["RAM", "SRAM"] {
            if core.memory(libretro::region_memory_id(r).unwrap()).is_some() {
                regions.push(r.to_string());
            }
        }
        if !core.has_connector() {
            eprintln!("note: this core has no coverage connector; replays will play but record no coverage");
        }
        (core.library_name.clone(), core.library_version.clone(), core.effective_options(), regions)
    };
    let mut syms = vec![];
    if let Some(s) = symbols {
        std::fs::create_dir_all(dir.join("symbols"))?;
        let fname = s.file_name().ok_or_else(|| anyhow!("bad symbols path"))?;
        std::fs::copy(s, dir.join("symbols").join(fname))?;
        let format = decomp_dj::analysis::guess_format(s);
        syms.push(SymbolsConfig { file: format!("symbols/{}", fname.to_string_lossy()), format: format.into(), rom_header: None, region: None, cpu_base: None });
    }
    let config = Config {
        format: 1,
        name: name.to_string(),
        rom: RomConfig { sha256: sha256_file(rom)?, file: rom.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() },
        core: CoreConfig { name: lib_name.to_lowercase(), build: core_build.unwrap_or_else(|| format!("{lib_name} {lib_version}")), sha256: String::new(), options: effective },
        ports: vec![PortConfig { port: 1, device: "JOYPAD".into() }],
        hash: HashConfig { algorithm: "blake3".into(), regions },
        tick: None,
        symbols: syms,
        cache: None,
        compare: vec![],
    };
    std::fs::write(dir.join("config.toml"), toml::to_string_pretty(&config)?)?;
    let col = Collection::open(dir)?;
    col.save_local(&LocalConfig { rom: Some(std::fs::canonicalize(rom)?), core: Some(std::fs::canonicalize(core_path)?), ..Default::default() })?;
    println!("created {}", dir.join("config.toml").display());
    Ok(())
}

fn new_replay(dir: &Path, parent: Option<String>, title: &str, frames: u64) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let parent = match parent {
        Some(p) => Some(col.resolve(&p)?.to_string()),
        None => None,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let r = Replay::new(&id, parent.as_deref(), title, frames);
    col.replays.insert(id.clone(), r);
    col.save_replay(&id)?;
    println!("{}", col.paths[&id].display());
    Ok(())
}

fn status(dir: &Path) -> Result<()> {
    let col = Collection::open(dir)?;
    let mut counts = BTreeMap::new();
    for (id, depth) in col.tree_order() {
        let r = &col.replays[id];
        let st = col.tree_status(id);
        *counts.entry(st.label()).or_insert(0) += 1;
        let mark = match st {
            Status::Clean => " ",
            Status::Dirty if r.get("end-hash").is_none() => "?",
            Status::Dirty => "~",
        };
        println!("{mark} {}{} {:>6}f  {}{}", "  ".repeat(depth), &id[..8], r.frames()?, r.title(), if r.is_poked() { "  [poked]" } else { "" });
    }
    println!("{} segments: {:?}  (? unrecorded: no end hash yet; ~ stale: starts where its parent no longer ends)", col.replays.len(), counts);
    Ok(())
}

/// Compares a result with the replay's recorded hashes. Returns the outcome label.
fn judge(col: &Collection, res: &ReplayResult) -> &'static str {
    let r = &col.replays[&res.id];
    if let Some(s) = r.get("start-hash") {
        if s != res.start_hash {
            return "dirty";
        }
    }
    let cps = r.checkpoints().unwrap_or_default();
    for (f, h) in &cps {
        if let Some((_, got)) = res.checkpoints.iter().find(|(gf, _)| gf == f) {
            if got != h {
                return "changed";
            }
        }
    }
    match r.get("end-hash") {
        None => "new",
        Some(e) if e == res.end_hash => "ok",
        Some(_) => "changed",
    }
}

fn accept(col: &mut Collection, res: &ReplayResult) -> Result<()> {
    let r = col.replays.get_mut(&res.id).unwrap();
    r.set("start-frame", &res.start_frame.to_string());
    r.set("start-hash", &res.start_hash);
    r.set("end-hash", &res.end_hash);
    if !res.checkpoints.is_empty() {
        r.remove_all("checkpoint");
        for (f, h) in &res.checkpoints {
            r.add("checkpoint", &format!("{f} {h}"));
        }
    }
    col.save_replay(&res.id)
}

fn save_shots(res: &ReplayResult, dir: &Path) -> Result<()> {
    for (f, frame) in &res.screenshots {
        let p = dir.join(format!("{}-{:06}.png", &res.id[..8], f));
        libretro::write_png(frame, &p)?;
        println!("  screenshot {}", p.display());
    }
    Ok(())
}

fn play(dir: &Path, id: &str, local: &Local, shots: Option<PathBuf>, extra: Vec<u64>, no_cache: bool, do_accept: bool) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let id = col.resolve(id)?.to_string();
    let results = {
        let mut s = open_session(&col, local)?;
        let opts = PlayOptions { end_screenshot: shots.is_some(), extra_shots: extra, record: true };
        s.play_chain(&col, &id, &opts, !no_cache)?
    };
    let mut bad = false;
    for res in &results {
        let outcome = judge(&col, res);
        println!("{:<8} {} {}  end {}", outcome, &res.id[..8], col.replays[&res.id].title(), &res.end_hash[..23]);
        if let Some(d) = &shots {
            save_shots(res, d)?;
        }
        if matches!(outcome, "dirty" | "changed") {
            bad = true;
        }
    }
    if do_accept {
        for res in &results {
            accept(&mut col, res)?;
        }
        println!("hashes written");
    } else if bad {
        bail!("hashes differ from the recorded ones (use --accept to record the new ones)");
    }
    Ok(())
}

fn run(dir: &Path, local: &Local, from_scratch: bool, do_accept: bool, screenshots: bool) -> Result<()> {
    let col = Collection::open(dir)?;
    let shot_dir = col.local_dir().join("screenshots");
    // What --accept needs, kept after the session (which borrows the collection) ends.
    let mut accepted: Vec<(ReplayResult, &'static str)> = vec![];
    let started = std::time::Instant::now();
    let mut total_frames = 0u64;
    {
        let mut s = open_session(&col, local)?;
        if !s.core.has_connector() {
            eprintln!("note: this core has no coverage connector; recording hashes only");
        }
        let run_id = match &s.cache {
            Some(c) => Some(c.begin_run(&col.config.core.build, if from_scratch { "run --from-scratch" } else { "run" })?),
            None => None,
        };
        let opts = PlayOptions { end_screenshot: screenshots, extra_shots: vec![], record: true };
        s.play_all(&col, &opts, from_scratch, |s, res| {
            let outcome = judge(&col, res);
            let depth = col.chain(&res.id)?.len() - 1;
            println!(
                "{:<8} {}{} {}  {}f{}",
                outcome,
                "  ".repeat(depth),
                &res.id[..8],
                col.replays[&res.id].title(),
                res.frames,
                res.ticks.map(|t| format!(", {t} ticks")).unwrap_or_default()
            );
            if screenshots {
                save_shots(res, &shot_dir)?;
            }
            if let (Some(c), Some(run)) = (&s.cache, run_id) {
                let key = s.content_key(&col, &res.id, &res.start_hash)?;
                c.run_result(run, &res.id, &key, outcome)?;
            }
            total_frames += res.frames;
            accepted.push((
                ReplayResult {
                    id: res.id.clone(),
                    start_frame: res.start_frame,
                    frames: res.frames,
                    start_hash: res.start_hash.clone(),
                    end_hash: res.end_hash.clone(),
                    checkpoints: res.checkpoints.clone(),
                    ticks: res.ticks,
                    poked: res.poked,
                    coverage: vec![],
                    screenshots: vec![],
                    end_state: vec![],
                },
                outcome,
            ));
            Ok(())
        })?;
    }
    let secs = started.elapsed().as_secs_f64();
    println!("played {total_frames} frames in {secs:.2}s ({:.0} frames/s)", total_frames as f64 / secs.max(1e-9));

    let mut col = col;
    let bad = accepted.iter().filter(|(_, o)| matches!(*o, "dirty" | "changed")).count();
    if do_accept {
        for (res, _) in &accepted {
            accept(&mut col, res)?;
        }
        println!("hashes written for {} replays", accepted.len());
    } else if bad > 0 {
        bail!("{bad} replays differ from their recorded hashes (use --accept to record the new ones)");
    }
    Ok(())
}

fn report(dir: &Path, local: &Local, json: Option<PathBuf>, gaps: Option<PathBuf>) -> Result<()> {
    let col = Collection::open(dir)?;
    let rom = match resolve_local(&col, local) {
        Ok((rom, _)) => Some(std::fs::read(&rom).with_context(|| format!("reading {}", rom.display()))?),
        Err(_) => None,
    };
    let conn = cache::Cache::open_readonly(&col)?;
    let rep = coverage::build_report(&col, &conn, rom.as_deref(), None)?;
    coverage::print_report(&rep, &col);
    if let Some(j) = json {
        std::fs::write(&j, serde_json::to_string_pretty(&rep)?)?;
        println!("wrote {}", j.display());
    }
    if let Some(g) = gaps {
        coverage::write_gaps_csv(&rep, &g)?;
        println!("wrote {}", g.display());
    }
    Ok(())
}

fn hydrate(dir: &Path, local: &Local, segment: Option<String>, children: bool, force: bool) -> Result<()> {
    if local.no_db {
        bail!("hydrate fills the local database, so it can't run with --no-db");
    }
    let mut col = Collection::open(dir)?;
    let seg = match &segment {
        Some(s) => Some(col.resolve(s)?.to_string()),
        None => None,
    };
    let mut h = Hydrator::new(&col, seg.as_deref().map(|s| (s, children)), force)?;
    let mut s = open_session(&col, local)?;
    let started = std::time::Instant::now();
    while !h.done() {
        for ev in h.step(&mut s, &mut col, std::time::Duration::from_secs(3600), 0)? {
            let depth = col.chain(&ev.id)?.len() - 1;
            println!("{:<7} {:<9} {}{} {}", ev.action, ev.verdict, "  ".repeat(depth), &ev.id[..8.min(ev.id.len())], col.replays[&ev.id].title());
        }
    }
    let played = h.events.iter().filter(|e| e.action == "played").count();
    println!("{played} played ({} frames in {:.1}s), {} already stored", h.frames_played, started.elapsed().as_secs_f64(), h.events.len() - played);
    let problems = h.problems();
    if !problems.is_empty() {
        println!("{} segments don't match their recorded hashes:", problems.len());
        for p in problems {
            println!("  {} {}: {}", &p.id[..8.min(p.id.len())], col.replays[&p.id].title(), p.verdict);
        }
    }
    Ok(())
}

/// `dj clean`: what Clean all segments does in the GUI, saving each segment it records.
fn clean(dir: &Path, local: &Local, segment: Option<String>, accept: bool, all: bool) -> Result<()> {
    if local.no_db {
        bail!("clean uses the local database, so it can't run with --no-db");
    }
    let mut col = Collection::open(dir)?;
    let seg = match &segment {
        Some(s) => Some(col.resolve(s)?.to_string()),
        None => None,
    };
    let mut h = Hydrator::verify(&col, seg.as_deref().map(|s| (s, true)), false, all)?;
    let mut s = open_session(&col, local)?;
    let started = std::time::Instant::now();
    while !h.done() {
        for ev in h.step(&mut s, &mut col, std::time::Duration::from_secs(3600), 0)? {
            if matches!(ev.verdict, "recorded" | "cleaned") || (accept && ev.verdict == "changed") {
                col.save_replay(&ev.id)?;
            } else if ev.verdict == "changed" {
                // Not written: put the file's version back, so the segments below wait for it.
                let p = col.paths[&ev.id].clone();
                col.replays.insert(ev.id.clone(), Replay::load(&p)?);
            }
            let depth = col.chain(&ev.id)?.len() - 1;
            println!("{:<7} {:<9} {}{} {}", ev.action, ev.verdict, "  ".repeat(depth), &ev.id[..8.min(ev.id.len())], col.replays[&ev.id].title());
        }
    }
    let n = |v: &str| h.events.iter().filter(|e| e.verdict == v).count();
    println!(
        "{} recorded, {} cleaned, {} with a changed ending, {} blocked, {} already clean ({} frames in {:.1}s)",
        n("recorded"),
        n("cleaned"),
        n("changed"),
        n("blocked"),
        n("ok"),
        h.frames_played,
        started.elapsed().as_secs_f64()
    );
    let changed: Vec<&str> = h.events.iter().filter(|e| e.verdict == "changed").map(|e| e.id.as_str()).collect();
    if !changed.is_empty() {
        println!("{}", if accept { "endings changed (written; check them, e.g. with git diff):" } else { "endings changed (not written; check them, then run again with --accept):" });
        for id in &changed {
            println!("  {} {}", &id[..8.min(id.len())], col.replays[*id].title());
        }
    }
    if n("blocked") > 0 {
        println!("blocked: segments below one that couldn't be cleaned (a segment above is still dirty)");
    }
    if !accept && !changed.is_empty() {
        bail!("{} segment(s) end differently from their files", changed.len());
    }
    Ok(())
}

fn snapshot_cmd(dir: &Path, segment: &str, frame: u64, out: &Path, local: &Local) -> Result<()> {
    let col = Collection::open(dir)?;
    let id = col.resolve(segment)?.to_string();
    let frames = col.replays[&id].frames()?;
    if frame > frames {
        bail!("{} has {frames} frames", &id[..8.min(id.len())]);
    }
    let mut s = open_session(&col, local)?;
    s.enter_replay(&col, &id)?;
    let start = s.state_hash(&col)?;
    let mut at = 0;
    if let Some((f, state)) = s.nearest_stored(&col, &id, &start, frame)? {
        if f > 0 {
            s.load_state(&state)?;
            at = f;
        }
    }
    s.restore_inputs(&col, &id, at)?;
    for f in at..frame {
        s.step(&col, &id, f, false)?;
    }
    let state = s.snapshot()?;
    std::fs::write(out, &state)?;
    println!("wrote {} ({} bytes; frame {frame} of {}, played {} frames from {})", out.display(), state.len(), &id[..8.min(id.len())], frame - at, if at == 0 { "the segment's start".to_string() } else { format!("the snapshot at frame {at}") });
    Ok(())
}

fn sql(dir: &Path, query: &str) -> Result<()> {
    let col = Collection::open(dir)?;
    let conn = cache::Cache::open_readonly(&col)?;
    let mut st = conn.prepare(query)?;
    let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    println!("{}", names.join("\t"));
    let n = names.len();
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        let mut cells = vec![];
        for i in 0..n {
            use rusqlite::types::ValueRef;
            cells.push(match row.get_ref(i)? {
                ValueRef::Null => "NULL".to_string(),
                ValueRef::Integer(v) => v.to_string(),
                ValueRef::Real(v) => v.to_string(),
                ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                ValueRef::Blob(b) => format!("<{} bytes>", b.len()),
            });
        }
        println!("{}", cells.join("\t"));
    }
    Ok(())
}

fn human(bytes: i64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.1} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

fn cache_stats(dir: &Path) -> Result<()> {
    let col = Collection::open(dir)?;
    let conn = cache::Cache::open_readonly(&col)?;
    let size = std::fs::metadata(cache::cache_path(&col)).map(|m| m.len()).unwrap_or(0);
    println!("{} ({})", cache::cache_path(&col).display(), human(size as i64));
    let settings = cache::CacheSettings::resolve(col.config.cache.as_ref(), col.settings()?.cache.as_ref())?;
    println!(
        "settings: preset {}, pages {}, a snapshot every {} frames{}, previews {:?}",
        settings.preset,
        settings.page_bytes.map(|b| format!("{} KB", b / 1024)).unwrap_or_else(|| "1:1".into()),
        settings.diff_every,
        if settings.diff_at_markers { " and at markers" } else { "" },
        settings.previews
    );
    for (name, q, bytes_q) in [
        ("contents", "SELECT count(*) FROM contents", None),
        ("snapshots", "SELECT count(*) FROM snapshots", Some("SELECT coalesce(sum(size),0) FROM snapshots")),
        ("pages", "SELECT count(*) FROM pages", Some("SELECT coalesce(sum(length(data)),0) FROM pages")),
        ("stored frames", "SELECT count(*) FROM frames", None),
        ("previews", "SELECT count(*) FROM images", Some("SELECT coalesce(sum(length(png)),0) FROM images")),
        ("coverage rows", "SELECT count(*) FROM coverage", Some("SELECT coalesce(sum(length(flags)),0) FROM coverage")),
        ("first hits", "SELECT count(*) FROM first_hits", None),
    ] {
        let n: i64 = conn.query_row(q, [], |r| r.get(0))?;
        let b = match bytes_q {
            Some(bq) => format!("  {}", human(conn.query_row(bq, [], |r| r.get(0))?)),
            None => String::new(),
        };
        println!("  {name:<14} {n:>8}{b}");
    }
    let raw: i64 = conn.query_row("SELECT coalesce(sum(size),0) FROM snapshots", [], |r| r.get(0))?;
    let stored: i64 = conn.query_row("SELECT coalesce(sum(length(data)),0) FROM pages", [], |r| r.get(0))?;
    if stored > 0 {
        println!("  snapshots take {} as pages, {:.1}x smaller than {} raw", human(stored), raw as f64 / stored as f64, human(raw));
    }
    if let Some(b) = settings.budget_bytes {
        if size > b {
            println!("  over the {} budget; run `dj cache gc`", human(b as i64));
        }
    }
    Ok(())
}

fn cache_gc(dir: &Path, local: &Local, force: bool) -> Result<()> {
    let col = Collection::open(dir)?;
    if !force && decomp_dj::cache::gui_open(&col.root) {
        bail!("a dj-gui has this project open, and gc would drop the snapshots of its unsaved edits; close it first, or pass --force");
    }
    let mut s = open_session(&col, local)?;
    let c = s.cache.as_mut().ok_or_else(|| anyhow!("gc needs the local database"))?;
    let before = c.file_size();
    let (contents, snaps, pages) = c.gc(&col)?;
    println!("removed {contents} contents, {snaps} snapshots and {pages} pages; {} -> {}", human(before as i64), human(c.file_size() as i64));
    Ok(())
}

