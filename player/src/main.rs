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
#[command(name = "dj", version, about = "Play trees of libretro input segments, check state hashes, and measure ROM code coverage")]
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
    /// Import a BizHawk TAS movie (.bk2) as a new root segment, and print its id
    ImportTas {
        dir: PathBuf,
        file: PathBuf,
        /// Title of the new segment (default: the game and the movie's author)
        #[arg(long)]
        title: Option<String>,
        /// Refuse a movie made with a different ROM, instead of warning
        #[arg(long)]
        strict: bool,
        /// ROM to check the movie against (default: the one in .local/local.toml)
        #[command(flatten)]
        local: Local,
    },
    /// Export the chain from the root segment down to SEGMENT as a BizHawk TAS movie (.bk2)
    ExportTas {
        dir: PathBuf,
        segment: String,
        /// The movie to write (.bk2)
        #[arg(long)]
        out: PathBuf,
        /// The last frame of SEGMENT to include, counted from its start (default: its last frame)
        #[arg(long)]
        through: Option<u64>,
        /// The movie's author
        #[arg(long)]
        author: Option<String>,
        /// Refuse a chain with what the movie can't carry (resets, [WRITE], [RNG], [DEVICE], a snapshot start), instead of leaving it out with a warning
        #[arg(long)]
        strict: bool,
        /// ROM the movie names (default: the one in .local/local.toml)
        #[command(flatten)]
        local: Local,
    },
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
    /// Developer tool: check the connector's coverage flags against the decoder, inside the
    /// symbol maps' functions (docs/cli.md)
    CheckConnector {
        dir: PathBuf,
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
    /// After rebuilding or updating the core: play every segment from power-on with it and,
    /// if every hash still matches, record its build in config.toml ([core] build)
    Repin {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
    },
    /// Rewrite every segment file in this dj's format (the first line, older field names) and
    /// record this dj in config.toml. What the segments play doesn't change
    Upgrade { dir: PathBuf },
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
    /// List annotations.toml's entries, and check them against the coverage
    Annotations {
        dir: PathBuf,
        #[command(flatten)]
        local: Local,
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
    // Started as a worker for code dj doesn't trust (.agents/ISOLATION_PLAN.md)?
    decomp_dj::isolate::run_if_worker();
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(2);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    // Batch commands play in a child dj, so a core that crashes or hangs costs a segment,
    // not the command (.agents/ISOLATION_PLAN.md).
    if let Cmd::Run { dir, .. } | Cmd::Clean { dir, .. } | Cmd::Hydrate { dir, .. } = &cli.cmd {
        if let Some(code) = decomp_dj::isolate::supervise(dir)? {
            std::process::exit(code);
        }
    }
    // A project a newer dj wrote, or settings this dj doesn't know: said once, here (in the
    // batch child when supervised, so not twice).
    if let Some(dir) = cmd_dir(&cli.cmd) {
        if let Ok(t) = decomp_dj::collection::layered_table(dir) {
            for w in decomp_dj::collection::check_versions(&t).unwrap_or_default() {
                eprintln!("warning: {w}");
            }
        }
    }
    match cli.cmd {
        Cmd::Init { dir, name, rom, core, core_build, options, symbols } => init(&dir, &name, &rom, &core, core_build, &options, symbols.as_deref()),
        Cmd::New { dir, parent, title, frames } => new_replay(&dir, parent, &title, frames),
        Cmd::Status { dir } => status(&dir),
        Cmd::ImportTas { dir, file, title, strict, local } => import_tas(&dir, &file, title.as_deref(), strict, &local),
        Cmd::ExportTas { dir, segment, out, through, author, strict, local } => export_tas(&dir, &segment, &out, through, author.as_deref(), strict, &local),
        Cmd::Play { dir, id, local, screenshots, shot, no_cache, accept } => play(&dir, &id, &local, screenshots, shot, no_cache, accept),
        Cmd::Run { dir, local, from_scratch, accept, screenshots } => run(&dir, &local, from_scratch, accept, screenshots),
        Cmd::Report { dir, local, json, gaps } => report(&dir, &local, json, gaps),
        Cmd::Annotations { dir, local } => annotations(&dir, &local),
        Cmd::Hydrate { dir, local, segment, children, force } => hydrate(&dir, &local, segment, children, force),
        Cmd::Clean { dir, local, segment, accept, all } => clean(&dir, &local, segment, accept, all),
        Cmd::Xref { dir, what, query } => queries::xref(&dir, &what, &query),
        Cmd::Summary { dir, query } => queries::summary(&dir, &query),
        Cmd::Gaps { dir, sort, only, query } => queries::gaps(&dir, sort, only, &query),
        Cmd::Function { dir, what, query } => queries::function(&dir, &what, &query),
        Cmd::Frontier { dir, what, query } => queries::frontier(&dir, what.as_deref(), &query),
        Cmd::CheckConnector { dir, query } => queries::check_connector(&dir, &query),
        Cmd::Snapshot { dir, segment, frame, out, local } => snapshot_cmd(&dir, &segment, frame, &out, &local),
        Cmd::Sql { dir, query } => sql(&dir, &query),
        Cmd::Worker { dir } => mcp::worker::run(dir),
        Cmd::Repin { dir, local } => repin(&dir, &local),
        Cmd::Upgrade { dir } => upgrade(&dir),
        Cmd::Mcp { dir, allow } => mcp::serve(dir, mcp::tools::Options { allow }),
        Cmd::Cache { cmd: CacheCmd::Stats { dir } } => cache_stats(&dir),
        Cmd::Cache { cmd: CacheCmd::Gc { dir, local, force } } => cache_gc(&dir, &local, force),
    }
}

/// The project a command works on, if any.
fn cmd_dir(cmd: &Cmd) -> Option<&Path> {
    Some(match cmd {
        Cmd::Init { .. } => return None,
        Cmd::ImportTas { dir, .. } | Cmd::ExportTas { dir, .. } => dir,
        Cmd::New { dir, .. } | Cmd::Status { dir } | Cmd::Play { dir, .. } | Cmd::Run { dir, .. } | Cmd::Report { dir, .. } | Cmd::Annotations { dir, .. } | Cmd::Hydrate { dir, .. } | Cmd::Clean { dir, .. } => dir,
        Cmd::Xref { dir, .. } | Cmd::Summary { dir, .. } | Cmd::Gaps { dir, .. } | Cmd::Function { dir, .. } | Cmd::Frontier { dir, .. } | Cmd::CheckConnector { dir, .. } => dir,
        Cmd::Snapshot { dir, .. } | Cmd::Sql { dir, .. } | Cmd::Worker { dir } | Cmd::Mcp { dir, .. } | Cmd::Repin { dir, .. } | Cmd::Upgrade { dir } => dir,
        Cmd::Cache { cmd: CacheCmd::Stats { dir } } | Cmd::Cache { cmd: CacheCmd::Gc { dir, .. } } => dir,
        #[allow(unreachable_patterns)]
        _ => return None,
    })
}

fn resolve_local(col: &Collection, local: &Local) -> Result<(PathBuf, PathBuf)> {
    let saved = col.settings()?;
    let rom = local.rom.clone().or(saved.rom).ok_or_else(|| anyhow!("no ROM given; pass --rom (it is then remembered)"))?;
    let core = local.core.clone().or(saved.core).ok_or_else(|| anyhow!("no core given; pass --core (it is then remembered)"))?;
    if local.rom.is_some() || local.core.is_some() {
        let mut saved = col.load_local()?;
        saved.rom = Some(std::fs::canonicalize(&rom).with_context(|| format!("the ROM {}", rom.display()))?);
        let lib = libretro::library_path(&core);
        saved.core = Some(std::fs::canonicalize(&lib).with_context(|| format!("the core library {}", lib.display()))?);
        col.save_local(&saved)?;
    }
    Ok((rom, core))
}

fn open_session(col: &Collection, local: &Local) -> Result<Session> {
    let (rom, core) = resolve_local(col, local)?;
    let use_db = !local.no_db && col.settings()?.database.unwrap_or(true);
    let s = Session::open_with(col, &rom, &core, use_db)?;
    if let Some(note) = &s.build_note {
        eprintln!("warning: {note}");
    }
    Ok(s)
}

fn init(dir: &Path, name: &str, rom: &Path, core_path: &Path, core_build: Option<String>, options: &[String], symbols: Option<&Path>) -> Result<()> {
    if dir.join("config.toml").exists() || dir.join("collection.toml").exists() {
        bail!("{} already has a project", dir.display());
    }
    let core_path = &libretro::library_path(core_path);
    if !core_path.is_file() {
        bail!("the core library {} doesn't exist", core_path.display());
    }
    let mut overrides = BTreeMap::new();
    for o in options {
        let (k, v) = o.split_once('=').ok_or_else(|| anyhow!("--option takes KEY=VALUE"))?;
        overrides.insert(k.to_string(), v.to_string());
    }
    std::fs::create_dir_all(dir.join("segments"))?;
    std::fs::create_dir_all(dir.join("blobs"))?;
    let work = dir.join(".local");
    let (lib_name, build_id, effective, regions) = {
        let mut core = libretro::Core::load(core_path, &work, &overrides)?;
        core.load_game(rom)?;
        // Some cores set up their memory on the first frame (Mupen64Plus-Next): run one so
        // the RAM regions to hash exist when they're probed.
        core.run_frame(false);
        let mut regions = vec![];
        for r in ["RAM", "SRAM"] {
            if core.memory(libretro::region_memory_id(r).unwrap()).is_some() {
                regions.push(r.to_string());
            }
        }
        if !core.has_connector() {
            eprintln!("note: this core has no coverage connector; replays will play but record no coverage");
        } else {
            print_connector_regions(&mut core, rom)?;
        }
        (core.library_name.clone(), core.build_id(), core.effective_options(), regions)
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
        decoders: vec![],
        dj: Some(decomp_dj::collection::DJ_VERSION.to_string()),
        format: decomp_dj::collection::PROJECT_FORMAT,
        name: name.to_string(),
        rom: RomConfig { sha256: sha256_file(rom)?, file: rom.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() },
        core: CoreConfig { name: lib_name.to_lowercase(), build: core_build.unwrap_or(build_id), sha256: String::new(), options: effective },
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

/// The project's ROM, for checking a movie against or naming in one: `--rom`, else the one
/// in the settings. None when neither is set.
fn tas_rom(col: &Collection, local: &Local) -> Result<Option<Vec<u8>>> {
    match local.rom.clone().or(col.settings()?.rom) {
        Some(p) => Ok(Some(std::fs::read(&p).with_context(|| format!("reading the ROM {}", p.display()))?)),
        None => Ok(None),
    }
}

fn import_tas(dir: &Path, file: &Path, title: Option<&str>, strict: bool, local: &Local) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let rom = tas_rom(&col, local)?;
    let ctx = decomp_dj::tas::Ctx { col: &col, rom: rom.as_deref() };
    let imp = decomp_dj::tas::import(&ctx, file, title, strict)?;
    for w in &imp.warnings {
        eprintln!("warning: {w}");
    }
    let id = imp.replay.id()?.to_string();
    let frames = imp.replay.frames()?;
    let lines = imp.replay.lines.len();
    col.replays.insert(id.clone(), imp.replay);
    col.save_replay(&id)?;
    eprintln!(
        "imported {} frames ({lines} lines, {} subtitles) from {} as a root segment; play it (dj play {} {}) to record its hashes and see how far it stays in sync",
        frames,
        imp.movie.markers.len(),
        imp.movie.meta.emulator,
        dir.display(),
        &id[..8]
    );
    println!("{id}");
    Ok(())
}

fn export_tas(dir: &Path, segment: &str, out: &Path, through: Option<u64>, author: Option<&str>, strict: bool, local: &Local) -> Result<()> {
    let col = Collection::open(dir)?;
    decomp_dj::tas::check_ext(out)?;
    let id = col.resolve(segment)?.to_string();
    let rom = tas_rom(&col, local)?;
    let ctx = decomp_dj::tas::Ctx { col: &col, rom: rom.as_deref() };
    let ex = decomp_dj::tas::export(&ctx, decomp_dj::tas::ExportEnd { segment: &id, through }, strict, author)?;
    for w in &ex.warnings {
        eprintln!("warning: {w}");
    }
    std::fs::write(out, &ex.bytes).with_context(|| format!("writing {}", out.display()))?;
    eprintln!("wrote {} frames ({} segment{}) to {}", ex.frames, ex.segments, if ex.segments == 1 { "" } else { "s" }, out.display());
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

fn annotations(dir: &Path, local: &Local) -> Result<()> {
    use decomp_dj::annotations::{Annotations, Kind};
    let col = Collection::open(dir)?;
    let ann = Annotations::load(&col.root)?;
    if ann.is_empty() {
        println!("no annotations ({} not found or empty)", col.root.join(decomp_dj::annotations::FILE).display());
        return Ok(());
    }
    let rom = match resolve_local(&col, local) {
        Ok((rom, _)) => Some(std::fs::read(&rom).with_context(|| format!("reading {}", rom.display()))?),
        Err(_) => None,
    };
    // What the maps alone say, to tell a rename from a new function.
    let maps = decomp_dj::analysis::project_functions(&col, rom.as_deref(), &Annotations::default())?;
    let merged = decomp_dj::analysis::project_functions(&col, rom.as_deref(), &ann)?;
    let rep = match cache::Cache::open_readonly(&col) {
        Ok(conn) => match coverage::build_report(&col, &conn, rom.as_deref(), None) {
            Ok(r) => Some(r),
            Err(e) => {
                println!("(not checked against coverage: {e:#})");
                None
            }
        },
        Err(e) => {
            println!("(not checked against coverage: {e:#})");
            None
        }
    };
    let mut problems = 0;
    for kind in [Kind::Unreachable, Kind::Data, Kind::Function] {
        for e in ann.of_kind(kind) {
            let mut line = format!("{:<20} {:<22}", e.label(), e.range_text());
            match kind {
                Kind::Function => {
                    let name = e.name.as_deref().unwrap_or("");
                    let was = maps.iter().find(|(f, _)| f.region == e.region && f.offset == e.start);
                    let now = merged.iter().find(|(f, _)| f.region == e.region && f.offset == e.start).map(|(f, _)| f.size).unwrap_or(0);
                    match was {
                        Some((f, _)) if f.name == name && f.size == now => line += &format!(" {name}: the map says the same"),
                        Some((f, _)) => line += &format!(" {name}: was {} ({:#X} bytes), now {now:#X} bytes", f.name, f.size),
                        None => line += &format!(" {name}: new, {now:#X} bytes"),
                    }
                }
                _ => {
                    let size = e.end - e.start + 1;
                    line += &format!(" {size:>5} bytes");
                    if let Some(w) = rep.as_ref().and_then(|r| r.annotation_warnings.iter().find(|w| w.entry == e.label())) {
                        line += &format!("  RAN: {}", w.text(&col));
                        problems += 1;
                    }
                }
            }
            if !e.why.is_empty() {
                line += &format!("  # {}", e.why);
            }
            println!("{line}");
        }
    }
    if let Some(r) = &rep {
        let t = &r.totals;
        println!("left out of coverage: {} unreachable {}, {} data bytes", t.unreachable_instructions, if r.symbols { "instructions" } else { "bytes" }, t.data_bytes);
    }
    if problems > 0 {
        bail!("{problems} annotated range(s) ran as code");
    }
    Ok(())
}

/// `dj repin`: checks the whole tree with the core as it is now, then records its build.
fn repin(dir: &Path, local: &Local) -> Result<()> {
    run(dir, local, true, false, false).context("not repinned: some segments don't reproduce their hashes with this core")?;
    let mut col = Collection::open(dir)?;
    let (_, core) = resolve_local(&col, local)?;
    let build = open_session(&col, local)?.build.clone();
    let sha = sha256_file(&libretro::library_path(&core))?;
    if build == col.config.core.build {
        println!("every segment matches; [core] build is already \"{build}\"");
        return Ok(());
    }
    let was = col.config.core.build.clone();
    col.set_core_build(&build, Some(&sha))?;
    println!("every segment matches: [core] build \"{was}\" -> \"{build}\" in config.toml{}", if col.config.core.sha256.is_empty() { "" } else { " (and its sha256)" });
    Ok(())
}

/// `dj upgrade`: every segment file and config.toml in this dj's format.
fn upgrade(dir: &Path) -> Result<()> {
    let mut col = Collection::open(dir)?;
    let ids: Vec<String> = col.tree_order().into_iter().map(|(id, _)| id.to_string()).filter(|id| !col.is_draft(id)).collect();
    let mut rewritten = 0;
    for id in &ids {
        let Some(path) = col.paths.get(id) else { continue };
        let disk = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if col.replays[id].to_text() != disk {
            col.save_replay(id)?;
            rewritten += 1;
        }
    }
    col.note_writer()?;
    println!(
        "{rewritten} of {} segment files rewritten (segment format {}); config.toml: format {}, dj {}",
        ids.len(),
        decomp_dj::format::SEGMENT_FORMAT,
        decomp_dj::collection::PROJECT_FORMAT,
        col.config.dj.as_deref().unwrap_or("?")
    );
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
    let mut conn = cache::Cache::open_readonly(&col)?;
    // Bring segments_now (and the v_ views on it) up to date with the segment files, for the
    // setup played last. A database nothing has been played into yet has no setup to use.
    let setup: Option<String> = rusqlite::OptionalExtension::optional(conn.query_row("SELECT setup FROM contents ORDER BY played_at DESC LIMIT 1", [], |r| r.get(0)))?;
    if let Some(setup) = setup {
        if let Err(e) = cache::refresh_segments_now(&mut conn, &col, &setup) {
            eprintln!("warning: couldn't bring segments_now up to date ({e:#}); the views show the segments as of the last play");
        }
    }
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


/// Prints what the connector reports about its regions (docs/connectors.md), and checks
/// that the bytes it hands out match the ROM file where it says they lie.
fn print_connector_regions(core: &mut libretro::Core, rom: &Path) -> Result<()> {
    let file = std::fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
    let facts = core.region_facts();
    let code = decomp_dj::analysis::code_region(&facts);
    println!("connector regions:");
    for (i, f) in facts.iter().enumerate() {
        let place = f.file_offset.map(|o| format!("ROM file from byte {o}")).unwrap_or_else(|| "not from the ROM file".into());
        let decoder = match f.cpu.as_deref() {
            Some(c) if decomp_dj::analysis::has_decoder(c) => format!("{c}"),
            Some(c) => format!("{c} (no built-in decoder: a plugin or the core can add one, docs/decoders.md)"),
            None => "no instruction set".into(),
        };
        let mark = if f.name == code { "  <- coverage rolls up here" } else { "" };
        println!("  {:<6} {:>8} bytes  {decoder}, {place}{mark}", f.name, f.size);
        if let (Some(off), Some(data)) = (f.file_offset, core.region_data(i)) {
            let off = off as usize;
            if file.get(off..off + data.len()) != Some(data) {
                eprintln!("warning: the connector's {} bytes don't match {} from byte {off}", f.name, rom.display());
            }
        }
    }
    Ok(())
}
