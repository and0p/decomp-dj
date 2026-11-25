//! dj-gui: browse a project's segment tree, play segments with live video, and
//! edit button inputs on a piano roll.
//!
//! Layout: menu bar on top, segment tree on the left, emulator preview in the
//! middle, piano roll along the bottom, status line at the very bottom.

// Windows: a windowed program, so no console window opens beside the GUI. Release builds
// only: a debug build keeps its console, for what dj-gui prints to stderr. (The workers dj-gui
// starts are dj-gui too, so they open no console either; their pipes work the same.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::Result;
use eframe::egui::{self, Color32};
use dj_core::collection::Collection;
use dj_core::compare::Comparison;
use dj_core::format::InputKey;
use dj_core::hydrate::Hydrator;
use dj_core::player::{PlayOptions, Recorder, Session};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

mod agent;
mod agent_ui;
mod compare;
mod console;
mod cov;
mod dialogs;
mod edit;
mod jobs;
mod menu;
mod new_project;
mod nodes;
mod parity;
mod playback;
mod project;
mod record;
mod record_ui;
mod roll;
mod save;
mod stick;
mod strings;
mod transport;
mod tree;
mod view;
mod widgets;

use strings as t;


/// A save state is kept every this many frames, so seeking never replays more than this.
const SNAP_EVERY: u64 = 10;
/// Agent drafts in the tree.
const DRAFT: Color32 = Color32::from_rgb(90, 200, 200);
/// Title of a draft an agent session has open (agent-owned, read-only here).
const AGENT_OWNED: Color32 = Color32::from_rgb(160, 205, 255);
/// The "!" on a segment whose ending changed since it was last saved.
const CHANGED: Color32 = Color32::from_rgb(235, 120, 90);

fn main() -> eframe::Result {
    // Started as a worker for code dj doesn't trust (.agents/_archive/ISOLATION_PLAN.md)?
    dj_core::isolate::run_if_worker();
    let start = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 560.0])
            .with_title("decomp-dj")
            // The window, taskbar (Windows) and Dock (macOS) icon. On Wayland the desktop shows
            // the icon of the desktop entry with this app id (crates/dj-gui/linux/).
            .with_app_id("decomp-dj")
            .with_icon(std::sync::Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon_gui.png")).expect("assets/icon_gui.png is a PNG"))),
        ..Default::default()
    };
    eframe::run_native(
        "decomp-dj",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            // No shading at the top and bottom of scrolled lists (the tree, the console).
            cc.egui_ctx.all_styles_mut(|s| s.spacing.scroll.fade.strength = 0.0);
            CTX.with(|c| *c.borrow_mut() = Some(cc.egui_ctx.clone()));
            let mut app = App::default();
            if let Some(p) = start {
                app.open_collection(&p);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Where playback stands inside the selected segment.
struct Playback {
    id: String,
    /// Frame of power-on at which this segment starts.
    start_frame: u64,
    frames: u64,
    /// Frames of this segment run so far. The picture shown is frame `ran - 1`.
    ran: u64,
    /// Save states at the start of frame N (before frame N's lines apply).
    snapshots: BTreeMap<u64, Vec<u8>>,
    checkpoints: BTreeMap<u64, String>,
    /// First checkpoint whose hash did not match, if any.
    checkpoint_miss: Option<u64>,
    /// Hashes this playback saw at the recorded checkpoint frames.
    checkpoints_seen: BTreeMap<u64, String>,
    /// State hash after the last frame, once playback has reached it.
    end_hash: Option<String>,
    /// State hash at frame 0, before any of this segment's lines.
    start_hash: Option<String>,
    /// Frames with a snapshot in the local database, for this content.
    stored: Vec<u64>,
    /// First frame not run again since the last edit: from here to the end, what the
    /// roll's ruler and the picture show may be out of date.
    stale_from: Option<u64>,
    /// The picture is from before the latest edit and is being recomputed.
    picture_stale: bool,
    /// The pass being recorded to the local database (snapshots, previews, coverage, first
    /// hits), while playback runs straight on from frame 0 of this content. A jump or an
    /// edit ends it, keeping the snapshots it took.
    rec: Option<Recorder>,
}

/// What the "!" compares with the saved file: the end hash, and for a root segment its
/// start hash too (a root that starts somewhere new is a change the author should see).
/// None until an end hash is recorded.
fn ending_sig(r: &dj_core::format::Segment) -> Option<String> {
    let end = r.get("end-hash")?;
    Some(match (r.parent(), r.get("start-hash")) {
        (None, Some(start)) => format!("{start} {end}"),
        _ => end.to_string(),
    })
}

/// Starts recording a pass of `id` from the core's current state, its frame 0.
fn start_pass(s: &mut Session, col: &Collection, id: &str, start_frame: u64) -> Result<Recorder> {
    s.begin(col, id, start_frame, &PlayOptions { record: true, ..Default::default() })
}

impl Playback {
    fn shown(&self) -> Option<u64> {
        self.ran.checked_sub(1)
    }
}

/// What the Save dialog asks.
/// What a clean plays.
#[derive(Clone, Copy, PartialEq)]
enum Clean {
    /// Only segments that aren't clean (before saving).
    Unclean,
    /// Those and the clean ones whose content isn't in the local database yet.
    Fill,
    /// Every segment, stored or not.
    Everything,
}

/// What waits on "discard unsaved edits?" (Open, Reload or Quit with edits not saved).
#[derive(Clone)]
enum Discard {
    Open(PathBuf),
    Reload,
    Quit,
}

impl Discard {
    fn verb(&self) -> &'static str {
        match self {
            Discard::Open(_) => t::OPEN_THE_OTHER_PROJECT,
            Discard::Reload => t::RELOAD_EVERY_SEGMENT_FROM_DISK,
            Discard::Quit => t::DISCARD_QUIT,
        }
    }

    fn button(&self) -> &'static str {
        match self {
            Discard::Open(_) => t::OPEN,
            Discard::Reload => t::RELOAD_ALL,
            Discard::Quit => t::QUIT,
        }
    }
}

enum SaveAsk {
    /// Saving would leave this many segments dirty: clean first?
    Dirty(usize),
    /// Cleaning (from the first question) is done: what to review before saving.
    AfterClean { changed: usize, dirty: usize, failed: Option<String> },
}

/// The dialogs that are open, or waiting on an answer.
struct Dialogs {
    node: Option<NodeDialog>,
    /// Inline rename in the tree: segment id, text, and whether focus is still to be given.
    renaming: Option<(String, String, bool)>,
    /// The segment waiting for delete confirmation.
    delete: Option<String>,
    /// Move to…: the segment being moved and the text filtering the list of new parents.
    move_to: Option<(String, String)>,
    /// Duplicate of a segment with children, waiting on whether to copy them too.
    dup: Option<String>,
    /// New root segment from here, waiting on the warning about snapshots: whether "Don't
    /// show this again" is ticked.
    snapshot_warning: Option<bool>,
    /// Save was asked for while it would leave this many segments dirty: offer to clean first.
    save: Option<SaveAsk>,
    /// Open, Reload or Quit asked for with unsaved edits: waiting on "discard them?".
    discard: Option<Discard>,
    /// File > New project…
    new_project: Option<new_project::NewProject>,
    /// Join with parent, waiting on its confirmation: (segment id, leave a marker, marker text).
    join: Option<(String, bool, String)>,
    /// Join chain, waiting on its confirmation: (top segment id, leave markers).
    chain: Option<(String, bool)>,
    frame: Option<FrameDialog>,
    /// File > Keyboard mapping…
    mapping: Option<record::MappingDialog>,
    /// A comparison core that failed to load: the message for its dialog.
    compare: Option<String>,
}

impl Default for Dialogs {
    fn default() -> Self {
        Dialogs {
            node: None,
            renaming: None,
            delete: None,
            move_to: None,
            dup: None,
            snapshot_warning: None,
            save: None,
            discard: None,
            new_project: None,
            join: None,
            chain: None,
            frame: None,
            mapping: None,
            compare: None,
        }
    }
}

/// The left sidebar: which list it shows, and how the tree of segments is filtered and folded.
struct SideState {
    closed: HashSet<String>,
    /// Tree filter text: matches title, id or tags; `#tag` matches that tag exactly.
    filter: String,
    /// While filtering, also show everything below each match.
    filter_below: bool,
    /// Which list the left sidebar shows.
    tab: SideTab,
    marker_filter: String,
    /// The segment the tree is focused on (hoisted): only it and its descendants show.
    hoist: Option<String>,
    /// Indent every level of the tree, instead of keeping only children in their parent's column.
    indent_all: bool,
}

impl Default for SideState {
    fn default() -> Self {
        SideState {
            closed: HashSet::new(),
            filter: String::new(),
            filter_below: false,
            tab: SideTab::Segments,
            marker_filter: String::new(),
            hoist: None,
            indent_all: false,
        }
    }
}

/// How playback runs: its speed, what it does at a segment's end, and the route it is playing along.
struct PlayState {
    clock: f64,
    /// Frames per tick multiplier; 0 means as fast as possible.
    /// Index into SPEEDS.
    speed: usize,
    /// What playback does at the end of a segment.
    end_mode: EndMode,
    /// Pause playback when a segment records an ending that differs from its saved file.
    stop_on_changed: bool,
    /// Sticky children: remember the deepest segment visited on the selected chain.
    /// The deepest segment visited on the current chain (tracked even while sticky is off).
    sticky_tip: Option<String>,
    /// "Play to": segments still to play through after the current one, and the last of them.
    route: Vec<String>,
    route_target: Option<String>,
}

impl Default for PlayState {
    fn default() -> Self {
        PlayState {
            clock: 0.0,
            speed: 3,
            end_mode: EndMode::Next,
            stop_on_changed: false,
            sticky_tip: None,
            route: vec![],
            route_target: None,
        }
    }
}

/// Undo and redo, the clipboard, and where the current run of edits started.
struct History {
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    clip: Option<Clip>,
    /// Return to the edit: an edit pauses playback and shows the earliest frame edited
    /// since playback last ran.
    return_to_edit: bool,
    /// The earliest frame of the current segment edited since playback last ran.
    edit_from: Option<u64>,
    /// The frame the last edit returned to. Showing any other frame (a seek, a step, a click
    /// on the ruler) starts a new run of edits.
    edit_at: Option<u64>,
}

impl Default for History {
    fn default() -> Self {
        History {
            undo: vec![],
            redo: vec![],
            clip: None,
            return_to_edit: true,
            edit_from: None,
            edit_at: None,
        }
    }
}

/// View > Debug console.
struct ConsoleState {
    /// View > Debug console: the log panel below the coverage map (off when the app starts).
    show: bool,
    filter: String,
    /// Which levels the console shows: info, warnings, errors.
    levels: [bool; 3],
    /// How many lines the console had when it was last drawn.
    seen: u64,
}

impl Default for ConsoleState {
    fn default() -> Self {
        ConsoleState {
            show: false,
            filter: String::new(),
            levels: [true; 3],
            seen: 0,
        }
    }
}

/// What is kept in `.local/recovery/` in case dj-gui doesn't close normally.
struct Recovery {
    /// What a dj-gui that crashed, hung or was force-quit left (unsaved edits, a crash
    /// note): offered back when the project opens (.agents/_archive/ISOLATION_PLAN.md, step 4).
    offer: Vec<dj_core::recovery::Left>,
    /// The unsaved edits last written to `.local/recovery/` (as JSON), and when that was checked.
    written: String,
    checked: f64,
}

impl Default for Recovery {
    fn default() -> Self {
        Recovery {
            offer: vec![],
            written: String::new(),
            checked: 0.0,
        }
    }
}

/// Agents working on the project through `dj mcp`: their drafts and sessions, and what is shared with them.
struct Agents {
    /// The MCP menu's switch: drafts, agent sessions and sharing are off until it is on
    /// (`[mcp] gui` in .local/local.toml).
    mcp_on: bool,
    /// Agent drafts from .local/drafts/ in the tree.
    show_drafts: bool,
    drafts_fp: u128,
    /// When the drafts folder and agent sessions were last looked at (UI time).
    polled: f64,
    sessions: Vec<agent::AgentSession>,
    /// Drafts taken over from their agent session, until the session is seen closed.
    taken_over: HashSet<String>,
    /// An agent session the view follows: (pid, session).
    follow: Option<(u64, String)>,
    /// Write what is shown to .local/mcp/gui.json for agents.
    share_view: bool,
    shared: Option<(String, u64)>,
}

impl Default for Agents {
    fn default() -> Self {
        Agents {
            mcp_on: false,
            show_drafts: true,
            drafts_fp: 0,
            polled: 0.0,
            sessions: vec![],
            taken_over: HashSet::new(),
            follow: None,
            share_view: false,
            shared: None,
        }
    }
}

/// Recording input from the keyboard.
struct RecState {
    /// Record mode (docs/gui.md#recording): frames played (real time) or committed (frame
    /// by frame) take the buttons held on the keyboard. Separate from playing or paused.
    recording: bool,
    /// The Record dropdown's options (`[record]` in .local/local.toml).
    cfg: record::Settings,
    /// Keys for the controller being recorded (`[keyboard]` in .local/local.toml).
    bindings: record::Bindings,
    /// The current recording pass has made its undo step. A pass runs from Record on to
    /// off, or over one held commit or rewind.
    pass: bool,
    /// Live coverage was restarted for the current pass.
    live: bool,
    /// Commit (true) or rewind held in frame-by-frame mode: seconds held, and the clock
    /// that paces repeats.
    hold: Option<(bool, f64, f64)>,
    /// Real-time recording: playback was running when rewind was pressed, so it carries on
    /// forward when rewind is let go.
    resume: bool,
}

impl Default for RecState {
    fn default() -> Self {
        RecState {
            recording: false,
            cfg: record::Settings::default(),
            bindings: record::Bindings::default(),
            pass: false,
            live: false,
            hold: None,
            resume: false,
        }
    }
}

/// The comparison core beside the baseline (docs/gui.md#comparison-cores): whether it is loaded, running and in step, the pre-roll before a seek, and parity results.
struct CompareState {
    /// The comparison core (docs/gui.md#comparison-cores), if one is selected and loaded.
    /// The comparison core, in this process beside the baseline
    /// (.agents/yield-parity/PLAN.md, section 11).
    core: Option<Comparison>,
    /// Where the comparison core is in step with the baseline (the segment and the next
    /// frame it runs; None: out of sync) and where it was last synced, where its run started.
    checker: dj_core::parity::Checker,
    /// Run comparison: off pauses a loaded comparison core without unloading it.
    run: bool,
    /// Try a sync at the next chance: set by jumps, Run comparison and Sync now; cleared by
    /// each attempt, so a failed sync is retried only at the next jump.
    need_sync: bool,
    /// Pre-roll ([[compare]] preroll, docs/gui.md): the frame a seek is going to show, when
    /// it started far enough back for the comparison core to be sent its jump early.
    preroll_to: Option<u64>,
    /// Re-running a failure: how many frames from where it was synced were pre-roll.
    rerun_preroll: Option<u64>,
    /// When the playhead last stepped back (egui's clock, seconds). For a moment after it
    /// the comparison core is not synced (`sync_held`), so that stepping back fast, or
    /// holding the key, doesn't sync at every step.
    stepped_back: Option<f64>,
    /// The baseline is not where the playback says: a pre-roll put it in a segment above and
    /// did not bring it back. The next seek loads a snapshot whatever frame it is for.
    displaced: bool,
    /// The next seek does no pre-roll in the segments above: the last one was given up.
    preroll_skip: bool,
    /// Why the comparison core is out of sync (a failed sync or step).
    problem: Option<String>,
    texture: Option<egui::TextureHandle>,
    /// The divider between the baseline and comparison pictures, as a fraction of the width.
    split: f32,
    /// Check parity after every frame both cores run (off at launch, not saved).
    check_parity: bool,
    /// A parity failure or a failed tick stopped playback: `compare_problem` says why, the
    /// comparison picture stays faded, and the next frame run syncs first.
    failed: bool,
    /// Parity results with the loaded comparison core (docs/gui.md#parity).
    parity: Option<parity::Parity>,
    /// Re-running parity failures.
    rerun: Option<parity::Rerun>,
    /// The baseline's next frame when it last ran one: the segment and frame. Anything else
    /// next is a jump.
    base_next: Option<(String, u64)>,
    /// Re-run the frame shown from the nearest snapshot (`try_seek`), to sync the comparison
    /// core before it and draw its picture.
    reshow: bool,
}

impl Default for CompareState {
    fn default() -> Self {
        CompareState {
            core: None,
            checker: Default::default(),
            run: true,
            need_sync: false,
            preroll_to: None,
            rerun_preroll: None,
            stepped_back: None,
            displaced: false,
            preroll_skip: false,
            problem: None,
            texture: None,
            split: 0.5,
            check_parity: false,
            failed: false,
            parity: None,
            rerun: None,
            base_next: None,
            reshow: false,
        }
    }
}

struct App {
    /// View > Stick pad.
    pad: stick::StickPad,
    col: Option<Collection>,
    session: Option<Session>,
    core_error: Option<String>,
    fps: f64,
    aspect: f32,
    selected: Option<String>,
    pb: Option<Playback>,
    playing: bool,
    /// Long emulation work, run a slice per screen refresh so the window stays live.
    job: Option<Job>,
    texture: Option<egui::TextureHandle>,
    /// The Functions tab and live coverage.
    cov: cov::CovState,
    edited: BTreeSet<String>,
    /// The first frame edited in segments that lost their end hash this run of the program,
    /// so the ruler band starts there even after leaving the segment.
    dirty_from: HashMap<String, u64>,
    /// The end hash of each segment as its file on disk has it, for the "!" on changed endings.
    saved_end: HashMap<String, Option<String>>,
    /// Modification times of segments/, to pick up files changed by other tools.
    segments_fp: u128,
    roll: roll::RollState,
    message: Option<(String, bool)>,
    /// Quit was confirmed (or nothing was unsaved): let the window close.
    quit_ok: bool,
    /// Save once the cleaning started from the save dialog finishes.
    save_after_clean: bool,
    /// Text to put on the system clipboard at the end of this frame.
    ctx_copy: Option<String>,
    /// Hydration in progress (it has the core to itself until it ends).
    hydrating: Option<Hydrating>,
    /// Tells `dj cache gc` this project is open here.
    gui_mark: Option<dj_core::cache::GuiMark>,
    /// Segments whose current content is in the local database.
    stored_ids: HashSet<String>,
    /// Limit for in-memory snapshots of the current segment, in MB.
    max_memory_mb: u64,
    /// What the coverage map shows (its tabs; Combined at launch).
    map_view: cov::MapView,
    /// Running frames to reach a seek's target: only the target frame is drawn (and syncs
    /// the comparison core), even while checking parity.
    seeking: bool,
    /// The dialogs that are open, or waiting on an answer.
    dlg: Dialogs,
    /// The left sidebar: which list it shows, and how the tree of segments is filtered and folded.
    side: SideState,
    /// How playback runs: its speed, what it does at a segment's end, and the route it is playing along.
    play: PlayState,
    /// Undo and redo, the clipboard, and where the current run of edits started.
    history: History,
    /// View > Debug console.
    console: ConsoleState,
    /// What is kept in `.local/recovery/` in case dj-gui doesn't close normally.
    recovery: Recovery,
    /// Agents working on the project through `dj mcp`: their drafts and sessions, and what is shared with them.
    agents: Agents,
    /// Recording input from the keyboard.
    rec: RecState,
    /// The comparison core beside the baseline (docs/gui.md#comparison-cores): whether it is loaded, running and in step, the pre-roll before a seek, and parity results.
    cmp: CompareState,
}

/// What playback does when it reaches the end of a segment.
#[derive(Clone, Copy, PartialEq)]
enum EndMode {
    /// Stop.
    Stop,
    /// Play the same segment again from frame 0.
    Loop,
    /// Carry on into the sticky child, or the only child.
    Next,
    /// Carry on down the segment list, in the tree's order.
    List,
}

impl EndMode {
    const ALL: [EndMode; 4] = [EndMode::Stop, EndMode::Loop, EndMode::Next, EndMode::List];

    fn label(self) -> &'static str {
        match self {
            EndMode::Stop => t::STOP,
            EndMode::Loop => t::LOOP,
            EndMode::Next => t::NEXT,
            EndMode::List => t::LIST,
        }
    }

    fn tip(self) -> &'static str {
        match self {
            EndMode::Stop => t::STOP_AT_THE_END_OF,
            EndMode::Loop => t::PLAY_THE_SEGMENT_AGAIN_FROM,
            EndMode::Next => t::CARRY_ON_INTO_THE_NEXT,
            EndMode::List => t::CARRY_ON_DOWN_THE_SEGMENT,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SideTab {
    Segments,
    Markers,
    Coverage,
}

struct Hydrating {
    h: Hydrator,
    paused: bool,
    what: String,
    /// The targets as they were before, for the undo step the clean leaves.
    before: HashMap<String, dj_core::format::Segment>,
}

/// Work too long to do in one screen refresh.
enum Job {
    /// Run the selected segment forward until `to` frames have run.
    Seek { from: u64, to: u64 },
    /// Play ancestors (`queue`) without a cached state to reach the start of `id`. Each is
    /// recorded as it plays (`rec`), so its end state is in the database the next time.
    Enter { id: String, queue: Vec<String>, qi: usize, frame: u64, start_frame: u64, done: u64, total: u64, then_seek: Option<u64>, rec: Option<Recorder> },
    /// Pre-roll in the segments above the one shown (docs/gui.md#comparison-cores), unseen:
    /// the baseline plays the last frames of `above` (segment, from, to; in the order they
    /// play) with the comparison core told of them as pre-roll, and comes out at frame 0 of
    /// the segment shown, which then runs until `to` frames have. `frame`: the next frame of
    /// `above[ri]`; before the first span's start, the baseline is catching up from a stored
    /// snapshot, and the core is told nothing. `base`: frames since power-on at frame 0 of
    /// `above[ri]`. `keep`: store the state the first span starts at under this content key,
    /// so the next pre-roll here starts from it.
    Preroll { above: Vec<(String, u64, u64)>, ri: usize, frame: u64, base: u64, to: u64, keep: Option<String>, done: u64, total: u64 },
}

/// Time spent on a job per screen refresh.
/// Playback speeds: (factor, label); 0.0 is as fast as the core goes.
const SPEEDS: [(f64, &str); 8] = [(0.1, "0.1×"), (0.25, "0.25×"), (0.5, "0.5×"), (1.0, "1×"), (2.0, "2×"), (4.0, "4×"), (8.0, "8×"), (0.0, t::MAX)];
/// Record mode's colour.
const REC_RED: Color32 = Color32::from_rgb(210, 45, 45);
/// Holding commit or rewind repeats after this long, in seconds.
const REC_HOLD_DELAY: f64 = 0.3;
const JOB_SLICE: std::time::Duration = std::time::Duration::from_millis(25);
/// A pre-roll in the segment above that has to play this many frames to reach its start
/// stores the state there, for the next one.
const PREROLL_KEEP: u64 = 30;
/// After a step back, the comparison core syncs once this long has passed with no other.
const STEP_BACK_HOLD: f64 = 0.6;

/// One step of undo history: the affected segments as they were before the change
/// (None: the segment did not exist), and where the user was.
struct UndoStep {
    label: String,
    segments: Vec<(String, Option<dj_core::format::Segment>)>,
    selected: Option<String>,
    frame: Option<u64>,
}

const UNDO_LIMIT: usize = 200;


/// Copied notes: button, start relative to the leftmost copied note, and length.
struct Clip {
    notes: Vec<(InputKey, u64, u64)>,
}

struct NodeDialog {
    id: String,
    title: String,
    frames: u64,
    /// Space-separated tags.
    tags: String,
    /// A root segment: it may start from a snapshot and a save file.
    root: bool,
    /// The save file (`sram:`) and the snapshot (`snapshot:`) a root segment starts from.
    sram: StartFile,
    snapshot: StartFile,
    /// The snapshots and save files in the project's blobs/ when the dialog opened, to pick
    /// from.
    blobs: Vec<BlobChoice>,
}

/// A file in blobs/ as the properties dialog offers it.
struct BlobChoice {
    /// The content hash a header names it by.
    hash: String,
    /// `sram` or `snapshot`.
    field: &'static str,
    /// The file's name without the hash digits; the start of the hash when it has none.
    name: String,
    size: u64,
    /// Titles of the segments that start from it.
    users: Vec<String>,
}

impl BlobChoice {
    fn all(col: &Collection) -> Vec<BlobChoice> {
        col.blobs()
            .into_iter()
            .map(|b| BlobChoice {
                name: match dj_core::collection::blob_name_parts(&b.stem) {
                    Some((name, _)) => name.to_string(),
                    None if b.stem == b.hash => String::new(),
                    None => b.stem.clone(),
                },
                users: b.users.iter().map(|id| col.segments[id].title().to_string()).collect(),
                hash: b.hash,
                field: b.field,
                size: b.size,
            })
            .collect()
    }

    /// What the list shows: the name (or the hash's start) and the size.
    fn label(&self) -> String {
        let size = if self.size >= 1 << 20 { format!("{:.1} MB", self.size as f64 / (1 << 20) as f64) } else if self.size >= 1024 { format!("{:.1} KB", self.size as f64 / 1024.0) } else { format!("{} bytes", self.size) };
        match self.name.is_empty() {
            true => format!("hash {}…  ({size})", &self.hash[..self.hash.len().min(8)]),
            false => format!("{}  ({size})", self.name),
        }
    }
}

/// A start blob of a root segment, as the properties dialog has it now.
#[derive(Default)]
struct StartFile {
    /// Its blob id.
    blob: Option<String>,
    /// A file picked in the dialog, not written to blobs/ until Apply: (path, bytes).
    pick: Option<(PathBuf, Vec<u8>)>,
    /// What the file is called in blobs/, before the hash digits: editable, for a picked
    /// file and for the blob there already (a rename on Apply).
    name: String,
    /// `name` when the dialog opened.
    was: String,
    /// What is typed into the list of the project's blobs, to narrow it.
    filter: String,
}

impl StartFile {
    fn of(col: &Collection, r: &dj_core::format::Segment, field: &str) -> StartFile {
        let blob = r.get(field).map(String::from);
        let name = blob.as_ref().and_then(|b| col.blob_name_of(b, field)).unwrap_or_default();
        StartFile { blob, pick: None, was: name.clone(), name, filter: String::new() }
    }

    /// Back to nothing chosen. What is typed into the list stays.
    fn clear(&mut self) {
        *self = StartFile { filter: std::mem::take(&mut self.filter), ..Default::default() };
    }

    /// The blob id it names after Apply.
    fn blob_after(&self) -> Option<String> {
        match &self.pick {
            Some((_, data)) => Some(dj_core::collection::blob_id(data)),
            None => self.blob.clone(),
        }
    }
}

struct FrameDialog {
    frame: u64,
    /// (line number, command text) for the non-input lines on this frame.
    rows: Vec<(usize, String)>,
    new_text: String,
    /// Put the keyboard focus in the new-line field on the next draw.
    focus: bool,
}

impl Default for App {
    fn default() -> Self {
        App {
            col: None,
            session: None,
            core_error: None,
            fps: 60.0,
            aspect: 0.0,
            selected: None,
            pb: None,
            playing: false,
            job: None,
            texture: None,
            cov: cov::CovState::default(),
            edited: BTreeSet::new(),
            dirty_from: HashMap::new(),
            segments_fp: 0,
            saved_end: HashMap::new(),
            roll: roll::RollState::default(),
            message: None,
            quit_ok: false,
            save_after_clean: false,
            ctx_copy: None,
            hydrating: None,
            gui_mark: None,
            stored_ids: HashSet::new(),
            max_memory_mb: 512,
            map_view: cov::MapView::Combined,
            seeking: false,
            dlg: Dialogs::default(),
            side: SideState::default(),
            play: PlayState::default(),
            history: History::default(),
            console: ConsoleState::default(),
            recovery: Recovery::default(),
            agents: Agents::default(),
            rec: RecState::default(),
            cmp: CompareState::default(),
            pad: stick::StickPad::default(),
        }
    }
}

/// Keeps in-memory snapshots under `budget` bytes. Frame 0 always stays, and so do
/// anchors every 300 frames while they fit; the snapshots farthest from `at` go first.
fn trim_snapshots(snaps: &mut BTreeMap<u64, Vec<u8>>, at: u64, budget: u64) {
    let size = |m: &BTreeMap<u64, Vec<u8>>| m.values().map(|v| v.len() as u64).sum::<u64>();
    let mut total = size(snaps);
    if total <= budget {
        return;
    }
    let mut order: Vec<u64> = snaps.keys().copied().filter(|k| *k != 0).collect();
    // Non-anchors first, each group farthest from the playhead first.
    order.sort_by_key(|k| (k % 300 == 0, std::cmp::Reverse(k.abs_diff(at))));
    for k in order {
        if total <= budget {
            break;
        }
        if let Some(v) = snaps.remove(&k) {
            total -= v.len() as u64;
        }
    }
}

fn subtree(col: &Collection, id: &str) -> Vec<String> {
    let mut out = vec![id.to_string()];
    let mut i = 0;
    while i < out.len() {
        let kids: Vec<String> = col.children(Some(&out[i])).into_iter().map(String::from).collect();
        out.extend(kids);
        i += 1;
    }
    out
}

thread_local! {
    static CTX: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
}

fn egui_ctx() -> egui::Context {
    CTX.with(|c| c.borrow().clone().expect("egui context"))
}

/// A normal close: nothing for the next launch to recover.
impl Drop for App {
    fn drop(&mut self) {
        if let Some(col) = &self.col {
            dj_core::recovery::clear(&col.root);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        // Closing the window (its close button, Cmd+Q) with unsaved edits asks first.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.cmp.save_parity(self.session.as_ref(), true);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok && self.unsaved_edits() > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dlg.discard = Some(Discard::Quit);
        }
        self.shortcuts(&ctx);
        if let Some(t) = self.ctx_copy.take() {
            ctx.copy_text(t);
        }
        let dt = ctx.input(|i| i.unstable_dt).min(0.1) as f64;
        let now = ctx.input(|i| i.time);
        dj_core::crash::heartbeat();
        if now - self.recovery.checked > 1.0 {
            self.recovery.checked = now;
            self.write_recovery();
        }
        if now - self.agents.polled > 0.5 {
            self.agents.polled = now;
            if let Some(m) = &mut self.gui_mark {
                m.touch();
            }
            self.poll_segments();
            self.poll_agents();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(600));
        // Lines logged by other threads (workers, cores) show without waiting for input.
        if self.console.show {
            if dj_core::console::count() != self.console.seen {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        self.work();
        self.coverage_update(ctx.input(|i| i.time));
        self.play.update_sticky_tip(self.col.as_ref(), self.selected.as_deref());
        if self.job.is_some() || self.hydrating.as_ref().is_some_and(|h| !h.paused) {
            ctx.request_repaint();
        }
        self.record_keys(&ctx, dt);
        self.tick(dt);
        self.rerun_step();
        self.compare_idle(&ctx);
        if self.playing {
            ctx.request_repaint();
        }
        let title = match &self.col {
            Some(c) => format!("decomp-dj: {}{}", c.config.name, if self.unsaved().is_empty() { "" } else { " *" }),
            None => "decomp-dj".into(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").exact_size(24.0).show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tree").resizable(true).default_size(300.0).size_range(180.0..=600.0).show(ui, |ui| {
            // View > Debug console: the log, at the very bottom of the sidebar (below the map).
            if self.console.show {
                // The list fills the panel itself; asking for the space left on top of that would
                // make the panel a little taller every frame.
                egui::Panel::bottom("console").resizable(true).default_size(160.0).size_range(60.0..=700.0).show(ui, |ui| self.console.panel(ui, &mut self.ctx_copy));
            }
            // The coverage map stays at the bottom of the sidebar, whatever the tab.
            if self.col.is_some() && self.session.is_some() {
                if self.map_view == cov::MapView::Hidden {
                    egui::Panel::bottom("cov_map_hidden").exact_size(26.0).show(ui, |ui| self.map_tabs(ui));
                } else {
                    egui::Panel::bottom("cov_map").resizable(true).default_size(190.0).size_range(80.0..=600.0).show(ui, |ui| {
                        self.coverage_map(ui);
                        // Fill the panel: a panel shrinks to what its content uses, which
                        // would undo a drag that made it taller.
                        ui.allocate_space(egui::vec2(ui.available_width(), ui.available_height().max(0.0)));
                    });
                }
            }
            self.tree(ui)
        });
        let mut action = None;
        egui::Panel::bottom("roll").resizable(true).default_size(300.0).size_range(120.0..=900.0).show(ui, |ui| {
            if let (Some(col), Some(id)) = (&self.col, &self.selected) {
                let shown = self.pb.as_ref().and_then(|p| p.shown());
                let nb = roll::Neighbours {
                    prev: col.segments.get(id).and_then(|r| r.parent()).map(String::from),
                    next: self.next_neighbour(),
                    dirty: self.dirty_band(id),
                };
                // What the inputs are called: the core's names, and the project's over them.
                let controls = self.controls();
                action = self.roll.show(ui, col, &controls, id, shown, self.playing, &nb);
            } else {
                ui.label(t::SELECT_A_SEGMENT_TO_SEE);
            }
        });
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.dialogs(&ctx);
        self.stick_pad(&ctx);
        let (controls, port) = (self.controls(), self.rec.cfg.port);
        if let Some(d) = &mut self.dlg.mapping {
            match d.show(&ctx, &controls, port) {
                Some(record::MappingResult::Save(b)) => {
                    self.rec.bindings = b;
                    self.dlg.mapping = None;
                    if let Some(col) = &self.col {
                        let mut local = col.load_local().unwrap_or_default();
                        local.keyboard = Some(self.rec.bindings.to_local());
                        if let Err(e) = col.save_local(&local) {
                            self.error(e);
                        }
                    }
                }
                Some(record::MappingResult::Cancel) => self.dlg.mapping = None,
                None => {}
            }
        }

        match action {
            Some(roll::Action::Seek(f)) => {
                self.playing = false;
                self.seek(f);
            }
            Some(roll::Action::Edit { edits, from }) => self.apply_edit(t::EDIT_NOTES, edits, from),
            Some(roll::Action::EditAxes { edits, from }) => self.apply_axis_edit(edits, from),
            Some(roll::Action::OpenFrame(f)) => self.open_frame_dialog(f, ""),
            None => {}
        }
    }
}
