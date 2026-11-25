//! The command line as clap reads it: the commands, their flags, and the arguments they share.

use crate::*;

#[derive(Parser)]
#[command(name = "dj", version, about = "Play trees of libretro input segments, check state hashes, measure ROM code coverage, and check a decomp's parity with the original")]
pub(crate) struct Cli {
    /// The project folder (default: the nearest folder with a dj config.toml, from the
    /// current one up)
    #[arg(short = 'C', long = "project", global = true, value_name = "DIR")]
    pub(crate) project: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

/// The ROM, for commands that read it but run no core.
#[derive(clap::Args, Clone, Default)]
pub(crate) struct RomArg {
    /// ROM file (default: $DJ_ROM, else the one saved in .local/local.toml)
    #[arg(long)]
    pub(crate) rom: Option<PathBuf>,
}

impl RomArg {
    pub(crate) fn local(&self) -> Local {
        Local { rom: self.rom.clone(), core: None, no_db: false }
    }
}

/// The ROM and the core, for commands that run one.
#[derive(clap::Args, Clone, Default)]
pub(crate) struct CoreArgs {
    /// ROM file (default: $DJ_ROM, else the one saved in .local/local.toml). Given here, it
    /// is saved there
    #[arg(long)]
    pub(crate) rom: Option<PathBuf>,
    /// libretro core library (default: $DJ_CORE, else the one saved in .local/local.toml).
    /// Given here, it is saved there
    #[arg(long)]
    pub(crate) core: Option<PathBuf>,
}

impl CoreArgs {
    pub(crate) fn local(&self, no_db: bool) -> Local {
        Local { rom: self.rom.clone(), core: self.core.clone(), no_db }
    }
}

/// Where a command finds the ROM and the core (given as flags, else `Collection::settings`:
/// `DJ_ROM` and `DJ_CORE`, then .local/local.toml), and whether it plays without the local
/// database.
#[derive(Clone, Default)]
pub struct Local {
    pub rom: Option<PathBuf>,
    pub core: Option<PathBuf>,
    pub no_db: bool,
}

/// Which segments a command works on: these, those with these tags, and with --children
/// those below them. None of either: every one.
#[derive(clap::Args, Clone, Default)]
pub(crate) struct Pick {
    /// These segments (ids or unique prefixes); none: all of them
    #[arg(value_name = "SEGMENT")]
    pub(crate) segments: Vec<String>,
    /// Also the segments with this tag in their `tags:` header (repeatable: any of them)
    #[arg(long = "tag", value_name = "NAME")]
    pub(crate) tags: Vec<String>,
    /// Also every segment below each one chosen
    #[arg(long)]
    pub(crate) children: bool,
}

impl From<Pick> for parity_cmd::Pick {
    fn from(p: Pick) -> Self {
        parity_cmd::Pick { segments: p.segments, tags: p.tags, children: p.children }
    }
}

#[derive(clap::Args, Clone)]
pub(crate) struct QueryArgs {
    /// Print JSON instead of text
    #[arg(long)]
    pub(crate) json: bool,
    /// Rows to show
    #[arg(long, default_value_t = 20)]
    pub(crate) limit: usize,
    /// Skip this many rows (the `next_cursor` of a previous page)
    #[arg(long, default_value_t = 0)]
    pub(crate) cursor: usize,
    #[command(flatten)]
    pub(crate) rom: RomArg,
}

#[derive(Subcommand)]
pub(crate) enum Cmd {
    /// Create a project pinned to a ROM and a core, with every core option recorded
    Init {
        /// The folder to create the project in (default: one named after the ROM file,
        /// without its extension, in the current folder)
        dir: Option<PathBuf>,
        /// The project's name (default: the folder's name, else the ROM file's without its
        /// extension)
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        rom: PathBuf,
        /// libretro core library, or the name of one in a cores folder (`mesen_libretro`).
        /// Default: $DJ_CORE, else the core for the ROM's console (NES, GBA, N64), looked
        /// for in $DJ_CORES, cores/ next to dj, and build/cores/
        #[arg(long)]
        core: Option<PathBuf>,
        /// Free-form id of the exact core build (default: the core's own name and version)
        #[arg(long)]
        core_build: Option<String>,
        /// Core option overrides, KEY=VALUE
        #[arg(long = "option")]
        options: Vec<String>,
        /// Symbol map to copy into symbols/ (cc65 .dbg, splat symbol_addrs.txt, Mesen .mlb, GNU ld .map, ELF or CSV)
        #[arg(long)]
        symbols: Option<PathBuf>,
        /// Don't make a first root segment (60 frames from power-on, titled "Power-on")
        #[arg(long)]
        no_segment: bool,
        /// Make the project without asking first. On a terminal dj shows what it would
        /// make and asks once; when its input or output isn't a terminal it never asks
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Create an empty segment: below --parent, or a root segment, which starts from
    /// power-on unless --snapshot, --sram or --from says otherwise (those need the core, to
    /// check that it loads what they name)
    New {
        #[arg(long)]
        parent: Option<String>,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value_t = 60)]
        frames: u64,
        /// A root segment that starts from this save state of the project's core (as `dj
        /// snapshot` writes one). Copied into blobs/
        #[arg(long, value_name = "FILE", conflicts_with_all = ["parent", "from"])]
        snapshot: Option<PathBuf>,
        /// A root segment that starts with this save file (battery-backed save RAM), with
        /// or without --snapshot. Copied into blobs/
        #[arg(long, value_name = "FILE", conflicts_with_all = ["parent", "from"])]
        sram: Option<PathBuf>,
        /// With --snapshot: how many frames after power-on the state was taken (default 0).
        /// It only numbers the frames
        #[arg(long, value_name = "N", requires = "snapshot")]
        start_frame: Option<u64>,
        /// A root segment that starts where SEGMENT is at the start of frame --at: its
        /// state, the buttons and sticks held there, and the devices plugged in
        #[arg(long, value_name = "SEGMENT", requires = "at", conflicts_with = "parent")]
        from: Option<String>,
        /// With --from: the frame of SEGMENT, counted from its start
        #[arg(long, value_name = "FRAME", requires = "from")]
        at: Option<u64>,
        /// What the copy's file in blobs/ is called, before the 8 hash digits dj adds (default:
        /// the file's name, or with --from the segment's title and the frame)
        #[arg(long, value_name = "NAME", conflicts_with = "parent")]
        blob_name: Option<String>,
        #[command(flatten)]
        core: CoreArgs,
    },
    /// Show the segment tree with clean and dirty marks (no emulation)
    Status,
    /// Import a BizHawk TAS movie (.bk2) as a new root segment, and print its id
    ImportTas {
        file: PathBuf,
        /// Title of the new segment (default: the game and the movie's author)
        #[arg(long)]
        title: Option<String>,
        /// Refuse a movie made with a different ROM, instead of warning
        #[arg(long)]
        strict: bool,
        /// ROM to check the movie against
        #[command(flatten)]
        rom: RomArg,
    },
    /// Export the chain from the root segment down to SEGMENT as a BizHawk TAS movie (.bk2)
    ExportTas {
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
        /// ROM the movie names
        #[command(flatten)]
        rom: RomArg,
    },
    /// Play segments (every one, or those chosen, with the segments above them) and compare
    /// their hashes with the recorded ones; fails when one differs. Writes no file: `dj
    /// clean` records hashes
    Check {
        #[command(flatten)]
        core: CoreArgs,
        #[command(flatten)]
        pick: Pick,
        /// Run without the local database (.local/cache.db)
        #[arg(long)]
        no_db: bool,
        /// Play each chain from its root segment instead of from its parent's end state, or
        /// a stored state of a segment above (checks that states save and load exactly)
        #[arg(long)]
        from_power_on: bool,
        /// Save each segment's last frame (and any [SCREENSHOT] frames) as PNGs in this folder
        #[arg(long, value_name = "DIR")]
        screenshots: Option<PathBuf>,
        /// With --screenshots: also these frames of each segment (counted from its start)
        #[arg(long, value_delimiter = ',', requires = "screenshots")]
        shot: Vec<u64>,
        /// Also fail on a segment with no recorded hashes ("new": never cleaned), as CI
        /// should: it proves nothing
        #[arg(long)]
        strict: bool,
        /// Also write a JUnit XML report here, one test case per segment (default: in the
        /// folder $DJ_JUNIT names, if set)
        #[arg(long, value_name = "FILE")]
        junit: Option<PathBuf>,
    },
    /// Play, top down, the segments that aren't clean or whose content isn't in the local
    /// database, and write the hashes they reach into their files (as Clean all segments does
    /// in dj-gui, which leaves them unsaved instead). A changed ending is listed, not written,
    /// and the command fails, unless --accept
    Clean {
        #[command(flatten)]
        core: CoreArgs,
        /// Only this segment (id or unique prefix)
        segment: Option<String>,
        /// With a segment: also every segment below it
        #[arg(long, requires = "segment")]
        children: bool,
        /// Also write changed endings (and go on below them)
        #[arg(long)]
        accept: bool,
        /// Play segments again even if their content is stored
        #[arg(long)]
        force: bool,
    },
    /// Check segments against a comparison core ([[compare]]), and work with the results kept
    Parity {
        #[command(subcommand)]
        cmd: ParityCmd,
    },
    /// Which segments first run an address, at which frame, nearest snapshot first
    #[command(alias = "where")]
    Xref {
        /// PRG:0x1F3A, 0x1F3A (a PRG offset), $C123 (a CPU address) or a symbol name (name+0x10 works)
        what: String,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Coverage of the current, clean, saved segments: the headline numbers, and what was
    /// left out
    Summary {
        /// Also each segment's own share, and the smallest set of segments that keeps all
        /// the coverage
        #[arg(long)]
        segments: bool,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Functions with uncovered code, largest or nearest first
    Gaps {
        /// size (most uncovered instructions) or nearness (skipped by a covered branch first)
        #[arg(long, default_value = "size")]
        sort: dj_core::query::GapSort,
        /// all, never (functions never entered) or partial
        #[arg(long, default_value = "all")]
        only: dj_core::query::GapFilter,
        /// Write every uncovered block to this CSV file instead (needs a symbol map)
        #[arg(long, value_name = "FILE", conflicts_with = "json")]
        csv: Option<PathBuf>,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// One function: its blocks, which run, which segments enter it, its callers
    Function {
        what: String,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Covered branches and calls that lead into uncovered code, with the segments that reach them
    Frontier {
        /// An address in uncovered code, or a function; leave out for the whole ROM
        what: Option<String>,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Developer tool: check the connector's coverage flags against the decoder, inside the
    /// symbol maps' functions (docs/cli.md)
    ConnectorCheck {
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Serve the project to an agent over MCP (stdio); see docs/mcp.md
    Mcp {
        /// What the agent may do: query, draft (the default) or save
        #[arg(long)]
        allow: Option<mcp::tools::Allow>,
    },
    /// Internal: one core driven by `dj mcp` over a pipe
    #[command(hide = true)]
    Worker,
    /// After rebuilding or updating the core: play every segment from power-on with it and,
    /// if every hash still matches, record its build in config.toml ([core] build)
    Repin {
        #[command(flatten)]
        core: CoreArgs,
    },
    /// Rewrite every segment file in this dj's format (the first line, older field names) and
    /// record this dj in config.toml. What the segments play doesn't change
    Upgrade,
    /// List what each controller's inputs are called: the core's names, and the project's
    /// labels over them ([[ports]] labels and hide in config.toml)
    Controls {
        #[command(flatten)]
        core: CoreArgs,
    },
    /// Write the core state at a frame of a segment, to start a trace from
    Snapshot {
        segment: String,
        frame: u64,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        core: CoreArgs,
        /// Run without the local database (.local/cache.db)
        #[arg(long)]
        no_db: bool,
    },
    /// Write the game's save RAM (its battery save) at a frame of a segment: a save file
    /// `dj new --sram` can start a root segment from
    Sram {
        segment: String,
        frame: u64,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        core: CoreArgs,
        /// Run without the local database (.local/cache.db)
        #[arg(long)]
        no_db: bool,
    },
    /// The snapshots and save files in blobs/ that root segments start from
    Blob {
        #[command(subcommand)]
        cmd: BlobCmd,
    },
    /// Run an SQL query on the local database
    Sql { query: String },
    /// Local database size, or clean it up
    Cache {
        #[command(subcommand)]
        cmd: CacheCmd,
    },
    /// List annotations.toml's entries, and check them against the coverage
    Annotations {
        #[command(flatten)]
        rom: RomArg,
    },
}

#[derive(Subcommand)]
pub(crate) enum ParityCmd {
    /// Check segments, each from its own start, at every verdict the comparison core gives
    /// (at its breakpoints in the original, or at frame ends), stopping a segment at its
    /// first failure, as Check parity does in dj-gui. Segments already checked with this
    /// build are skipped (unless --again). Exits 1 when one fails
    Check {
        #[command(flatten)]
        core: CoreArgs,
        #[command(flatten)]
        pick: Pick,
        /// The [[compare]] entry to check against (needed when there are several)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
        /// Check segments again even if they were checked with this build
        #[arg(long)]
        again: bool,
        /// Also check segments that aren't clean
        #[arg(long)]
        dirty: bool,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
        /// One JSON line per segment as it finishes (for the MCP server's parity jobs)
        #[arg(long, hide = true)]
        events: bool,
        /// Also write a JUnit XML report here, one test case per segment (default: in the
        /// folder $DJ_JUNIT names, if set)
        #[arg(long, value_name = "FILE")]
        junit: Option<PathBuf>,
    },
    /// The results kept: instructions and functions by parity, and the failures with
    /// their ids. Runs no core
    Report {
        #[command(flatten)]
        rom: RomArg,
        /// The [[compare]] entry (needed when there are several)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
        /// Functions to list
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Play failures again, each from its last jump, with the build there is now: these ids,
    /// or with none every failure from an older build. One from an older build that passes
    /// is cleared. Exits 1 when one still fails
    Rerun {
        /// Failure ids (`dj parity report` lists them)
        ids: Vec<i64>,
        #[command(flatten)]
        core: CoreArgs,
        /// The [[compare]] entry (needed when there are several)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
        /// One JSON line per failure as it finishes (for the MCP server's parity jobs)
        #[arg(long, hide = true)]
        events: bool,
    },
    /// Forget failures. Runs no core
    Clear {
        /// Failure ids (`dj parity report` lists them)
        #[arg(required = true)]
        ids: Vec<i64>,
        /// The [[compare]] entry (needed when there are several)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
    },
    /// Play a failure again and say what differed, and which instruction of the original
    /// wrote each value (needs a trace hook in the baseline's connector). Keeps no results
    Explain {
        /// The failure's id (`dj parity report` lists them)
        id: i64,
        #[command(flatten)]
        core: CoreArgs,
        /// The [[compare]] entry (needed when there are several)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
        /// Also trace this many frames before the failing one
        #[arg(long, default_value_t = 10, value_name = "N")]
        frames_back: u64,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Load the comparison core and say what it is: how it follows the baseline, whether it
    /// follows [WRITE] lines, its pre-roll, its build, and its breakpoints in the original,
    /// each checked against the baseline connector's regions. Exits 1 when one can't be set
    Core {
        #[command(flatten)]
        core: CoreArgs,
        /// The [[compare]] entry (needed when there are several)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum CacheCmd {
    /// What the local database holds
    Stats,
    /// Drop content no current segment uses, and pages no snapshot uses. Refuses while a
    /// dj-gui has the project open (its unsaved edits' snapshots would go), unless --force
    Gc {
        #[command(flatten)]
        core: CoreArgs,
        /// Run even while a dj-gui has the project open
        #[arg(long)]
        force: bool,
    },
    /// Fill the local database: play the segments whose content isn't stored yet, top down,
    /// and write no file. Unlike `dj clean` it records no hashes, so it plays segments that
    /// aren't clean too, from wherever they start now
    Fill {
        #[command(flatten)]
        core: CoreArgs,
        /// Only this segment (id or unique prefix)
        segment: Option<String>,
        /// With a segment: also every segment below it
        #[arg(long, requires = "segment")]
        children: bool,
        /// Play segments again even if their content is stored
        #[arg(long)]
        force: bool,
    },
    /// Write the states that checking these segments starts from, one file each, named by
    /// state hash: for `dj cache seed` on another machine (CI: docs/ci.md). Each segment's
    /// start (its parent's end) and, with a [[compare]] preroll, the start of the segment its
    /// pre-roll begins in. They hold the game's memory: keep them where the ROM is kept
    Export {
        #[command(flatten)]
        core: CoreArgs,
        #[command(flatten)]
        pick: Pick,
        /// The [[compare]] entry whose preroll to allow for (default: the largest of all)
        #[arg(long, value_name = "NAME")]
        compare: Option<String>,
        /// The folder to write them into
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Store states written by `dj cache export`, checked by hash: each is loaded into the
    /// core and kept only if its state hash is a clean segment's recorded end hash. Exits 1
    /// when one is refused
    Seed {
        /// State files, or folders of them
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
        #[command(flatten)]
        core: CoreArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum BlobCmd {
    /// List the files in blobs/ with their size and the segments that start from them
    List,
    /// Give a blob's file another name. The 8 hash digits at the end stay, and no segment
    /// changes: segments name a blob by the hash of its content
    Rename {
        /// The file's name in blobs/ (`world-3-save-a1b2c3d4`), or the start of its name or hash
        blob: String,
        /// The new name, without the hash digits
        #[arg(value_name = "NAME")]
        name: String,
    },
}
