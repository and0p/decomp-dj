//! dj-gui: browse a project's segment tree, play segments with live video, and
//! edit button inputs on a piano roll.
//!
//! Layout: menu bar on top, segment tree on the left, emulator preview in the
//! middle, piano roll along the bottom, status line at the very bottom.

// Windows: a windowed program, so no console window opens beside the GUI. Release builds
// only: a debug build keeps its console, for what dj-gui prints to stderr. (The workers dj-gui
// starts are dj-gui too, so they open no console either; their pipes work the same.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{anyhow, Result};
use eframe::egui::{self, Color32, RichText};
use decomp_dj::collection::{Collection, LocalConfig, Status};
use decomp_dj::compare::RemoteComparison;
use decomp_dj::parity::Checked;
use decomp_dj::format::{Command, InputKey};
use decomp_dj::hydrate::Hydrator;
use decomp_dj::player::{trunk_start_frame, PlayOptions, Recorder, Session};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

mod agent;
mod cov;
mod parity;
mod record;
mod roll;

/// A save state is kept every this many frames, so seeking never replays more than this.
const SNAP_EVERY: u64 = 10;
/// Agent drafts in the tree.
const DRAFT: Color32 = Color32::from_rgb(90, 200, 200);
/// Title of a draft an agent session has open (agent-owned, read-only here).
const AGENT_OWNED: Color32 = Color32::from_rgb(160, 205, 255);
/// The "!" on a segment whose ending changed since it was last saved.
const CHANGED: Color32 = Color32::from_rgb(235, 120, 90);

fn main() -> eframe::Result {
    // Started as a worker for code dj doesn't trust (.agents/ISOLATION_PLAN.md)?
    decomp_dj::isolate::run_if_worker();
    let start = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 560.0])
            .with_title("decomp-dj")
            // The window, taskbar (Windows) and Dock (macOS) icon. On Wayland the desktop shows
            // the icon of the desktop entry with this app id (gui/linux/).
            .with_app_id("decomp-dj")
            .with_icon(std::sync::Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon_gui.png")).expect("assets/icon_gui.png is a PNG"))),
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

/// Where playback stands inside the selected replay.
struct Playback {
    id: String,
    /// Frame of power-on at which this replay starts.
    start_frame: u64,
    frames: u64,
    /// Frames of this replay run so far. The picture shown is frame `ran - 1`.
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
    /// State hash at frame 0, before any of this replay's lines.
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
fn ending_sig(r: &decomp_dj::format::Replay) -> Option<String> {
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
            Discard::Open(_) => "open the other project",
            Discard::Reload => "reload every segment from disk",
            Discard::Quit => "quit",
        }
    }

    fn button(&self) -> &'static str {
        match self {
            Discard::Open(_) => "Open",
            Discard::Reload => "Reload all",
            Discard::Quit => "Quit",
        }
    }
}

enum SaveAsk {
    /// Saving would leave this many segments dirty: clean first?
    Dirty(usize),
    /// Cleaning (from the first question) is done: what to review before saving.
    AfterClean { changed: usize, dirty: usize, failed: Option<String> },
}

struct App {
    col: Option<Collection>,
    session: Option<Session>,
    core_error: Option<String>,
    fps: f64,
    aspect: f32,
    selected: Option<String>,
    pb: Option<Playback>,
    playing: bool,
    clock: f64,
    /// Long emulation work, run a slice per screen refresh so the window stays live.
    job: Option<Job>,
    texture: Option<egui::TextureHandle>,
    tree_closed: HashSet<String>,
    /// Tree filter text: matches title, id or tags; `#tag` matches that tag exactly.
    filter: String,
    /// While filtering, also show everything below each match.
    filter_below: bool,
    /// Which list the left sidebar shows.
    side_tab: SideTab,
    /// The Functions tab and live coverage.
    cov: cov::CovState,
    marker_filter: String,
    /// The segment the tree is focused on (hoisted): only it and its descendants show.
    hoist: Option<String>,
    /// Indent every level of the tree, instead of keeping only children in their parent's column.
    indent_all: bool,
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
    node_dialog: Option<NodeDialog>,
    /// Inline rename in the tree: segment id, text, and whether focus is still to be given.
    renaming: Option<(String, String, bool)>,
    /// The segment waiting for delete confirmation.
    delete_dialog: Option<String>,
    /// Move to…: the segment being moved and the text filtering the list of new parents.
    move_dialog: Option<(String, String)>,
    /// Duplicate of a segment with children, waiting on whether to copy them too.
    dup_dialog: Option<String>,
    /// Save was asked for while it would leave this many segments dirty: offer to clean first.
    save_dialog: Option<SaveAsk>,
    /// Open, Reload or Quit asked for with unsaved edits: waiting on "discard them?".
    discard_dialog: Option<Discard>,
    /// What a dj-gui that crashed, hung or was force-quit left (unsaved edits, a crash
    /// note): offered back when the project opens (.agents/ISOLATION_PLAN.md, step 4).
    recovery_offer: Vec<decomp_dj::recovery::Left>,
    /// The unsaved edits last written to `.local/recovery/` (as JSON), and when that was checked.
    recovery_written: String,
    recovery_checked: f64,
    /// Quit was confirmed (or nothing was unsaved): let the window close.
    quit_ok: bool,
    /// Save once the cleaning started from the save dialog finishes.
    save_after_clean: bool,
    /// Join with parent, waiting on its confirmation: (segment id, leave a marker, marker text).
    join_dialog: Option<(String, bool, String)>,
    /// Join chain, waiting on its confirmation: (top segment id, leave markers).
    chain_dialog: Option<(String, bool)>,
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    clip: Option<Clip>,
    /// Text to put on the system clipboard at the end of this frame.
    ctx_copy: Option<String>,
    /// View > Debug console: the log panel below the coverage map (off when the app starts).
    show_console: bool,
    console_filter: String,
    /// Which levels the console shows: info, warnings, errors.
    console_levels: [bool; 3],
    /// How many lines the console had when it was last drawn.
    console_seen: u64,
    frame_dialog: Option<FrameDialog>,
    /// Hydration in progress (it has the core to itself until it ends).
    hydrating: Option<Hydrating>,
    /// Tells `dj cache gc` this project is open here.
    gui_mark: Option<decomp_dj::cache::GuiMark>,
    /// Segments whose current content is in the local database.
    stored_ids: HashSet<String>,
    /// Limit for in-memory snapshots of the current segment, in MB.
    max_memory_mb: u64,
    /// Return to the edit: an edit pauses playback and shows the earliest frame edited
    /// since playback last ran.
    return_to_edit: bool,
    /// The earliest frame of the current segment edited since playback last ran.
    edit_from: Option<u64>,
    /// The frame the last edit returned to. Showing any other frame (a seek, a step, a click
    /// on the ruler) starts a new run of edits.
    edit_at: Option<u64>,
    /// The MCP menu's switch: drafts, agent sessions and sharing are off until it is on
    /// (`[mcp] gui` in .local/local.toml).
    mcp_on: bool,
    /// Agent drafts from .local/drafts/ in the tree.
    show_drafts: bool,
    drafts_fp: u128,
    /// When the drafts folder and agent sessions were last looked at (UI time).
    agent_polled: f64,
    agent_sessions: Vec<agent::AgentSession>,
    /// Drafts taken over from their agent session, until the session is seen closed.
    taken_over: HashSet<String>,
    /// An agent session the view follows: (pid, session).
    follow: Option<(u64, String)>,
    /// Write what is shown to .local/mcp/gui.json for agents.
    share_view: bool,
    shared: Option<(String, u64)>,
    /// Record mode (docs/gui.md#recording): frames played (real time) or committed (frame
    /// by frame) take the buttons held on the keyboard. Separate from playing or paused.
    recording: bool,
    /// The Record dropdown's options (`[record]` in .local/local.toml).
    rec_cfg: record::Settings,
    /// Keys for the controller being recorded (`[keyboard]` in .local/local.toml).
    bindings: record::Bindings,
    /// The current recording pass has made its undo step. A pass runs from Record on to
    /// off, or over one held commit or rewind.
    rec_pass: bool,
    /// Live coverage was restarted for the current pass.
    rec_live: bool,
    /// Commit (true) or rewind held in frame-by-frame mode: seconds held, and the clock
    /// that paces repeats.
    rec_hold: Option<(bool, f64, f64)>,
    /// Real-time recording: playback was running when rewind was pressed, so it carries on
    /// forward when rewind is let go.
    rec_resume: bool,
    /// File > Keyboard mapping…
    mapping: Option<record::MappingDialog>,
    /// The comparison core (docs/gui.md#comparison-cores), if one is selected and loaded.
    /// The comparison core, in its own worker process (.agents/ISOLATION_PLAN.md).
    compare: Option<RemoteComparison>,
    /// Where the comparison core is in step with the baseline (the segment and the next
    /// frame it runs; None: out of sync) and where it was last synced, where its run started.
    checker: decomp_dj::parity::Checker,
    /// Run comparison: off pauses a loaded comparison core without unloading it.
    compare_run: bool,
    /// Try a sync at the next chance: set by jumps, Run comparison and Sync now; cleared by
    /// each attempt, so a failed sync is retried only at the next jump.
    compare_need_sync: bool,
    /// Why the comparison core is out of sync (a failed sync or step).
    compare_problem: Option<String>,
    compare_texture: Option<egui::TextureHandle>,
    /// The divider between the baseline and comparison pictures, as a fraction of the width.
    compare_split: f32,
    /// A comparison core that failed to load: the message for its dialog.
    compare_dialog: Option<String>,
    /// Check parity after every frame both cores run (off at launch, not saved).
    check_parity: bool,
    /// A parity failure or a failed tick stopped playback: `compare_problem` says why, the
    /// comparison picture stays faded, and the next frame run syncs first.
    compare_failed: bool,
    /// Parity results with the loaded comparison core (docs/gui.md#parity).
    parity: Option<parity::Parity>,
    /// What the coverage map shows (its tabs; Combined at launch).
    map_view: cov::MapView,
    /// Re-running parity failures.
    rerun: Option<parity::Rerun>,
    /// Running frames to reach a seek's target: only the target frame is drawn (and syncs
    /// the comparison core), even while checking parity.
    seeking: bool,
    /// The baseline's next frame when it last ran one: the segment and frame. Anything else
    /// next is a jump.
    base_next: Option<(String, u64)>,
    /// Re-run the frame shown from the nearest snapshot (`try_seek`), to sync the comparison
    /// core before it and draw its picture.
    reshow: bool,
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
            EndMode::Stop => "Stop",
            EndMode::Loop => "Loop",
            EndMode::Next => "Next",
            EndMode::List => "List",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            EndMode::Stop => "Stop at the end of each segment.",
            EndMode::Loop => "Play the segment again from its first frame, until you stop it.",
            EndMode::Next => "Carry on into the next segment: the child you came back up from (sticky), or the only child. Stops where a segment has several children and none is sticky, or where the child is open in an agent session.",
            EndMode::List => "Carry on down the segment list in tree order, through every segment below and after this one (whether folded or filtered out), to the end of the list. With the tree focused on a segment (Focus here), it stops at the end of that segment's branch. Started on a root segment, it plays the whole tree.",
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
    before: HashMap<String, decomp_dj::format::Replay>,
}

/// Work too long to do in one screen refresh.
enum Job {
    /// Run the selected segment forward until `to` frames have run.
    Seek { from: u64, to: u64 },
    /// Play ancestors (`queue`) without a cached state to reach the start of `id`.
    Enter { id: String, queue: Vec<String>, qi: usize, frame: u64, start_frame: u64, done: u64, total: u64, then_seek: Option<u64> },
}

/// Time spent on a job per screen refresh.
/// Playback speeds: (factor, label); 0.0 is as fast as the core goes.
const SPEEDS: [(f64, &str); 8] = [(0.1, "0.1×"), (0.25, "0.25×"), (0.5, "0.5×"), (1.0, "1×"), (2.0, "2×"), (4.0, "4×"), (8.0, "8×"), (0.0, "MAX")];
/// Record mode's colour.
const REC_RED: Color32 = Color32::from_rgb(210, 45, 45);
/// Holding commit or rewind repeats after this long, in seconds.
const REC_HOLD_DELAY: f64 = 0.3;
const JOB_SLICE: std::time::Duration = std::time::Duration::from_millis(25);

/// One step of undo history: the affected segments as they were before the change
/// (None: the segment did not exist), and where the user was.
struct UndoStep {
    label: String,
    segments: Vec<(String, Option<decomp_dj::format::Replay>)>,
    selected: Option<String>,
    frame: Option<u64>,
}

const UNDO_LIMIT: usize = 200;

const NEW_ROOT_TIP: &str = "A new empty segment with no parent: it starts from power-on (or from a save file set in its properties)";
const JOIN_CHAIN_TIP: &str = "Join every only-child below this segment into it, down to the first segment with no children or several, as one undo step";
const DUP_TIP: &str = "A copy of this segment beside it (same parent, same inputs); with children, it asks whether to copy them too";
const MOVE_TIP: &str = "Give this segment (and what is below it) another parent, or make it a root segment. It and everything below turn dirty.";
const JOIN_TIP: &str = "Merge this segment into its parent: the parent keeps its name and grows by this segment's frames. Only for a segment with no siblings.";
const JOIN_CHILD_TIP: &str = "Merge this segment's only child into it: this segment keeps its name and grows by the child's frames. Only for a segment with exactly one child.";

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
}

/// A start blob of a root segment, as the properties dialog has it now.
#[derive(Default)]
struct StartFile {
    /// Its blob id.
    blob: Option<String>,
    /// A file picked in the dialog, not written to blobs/ until Apply: (path, bytes).
    pick: Option<(PathBuf, Vec<u8>)>,
}

impl StartFile {
    fn of(r: &decomp_dj::format::Replay, field: &str) -> StartFile {
        StartFile { blob: r.get(field).map(String::from), pick: None }
    }

    /// The blob id it names after Apply.
    fn blob_after(&self) -> Option<String> {
        match &self.pick {
            Some((_, data)) => Some(decomp_dj::collection::blob_id(data)),
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
            clock: 0.0,
            job: None,
            texture: None,
            tree_closed: HashSet::new(),
            filter: String::new(),
            filter_below: false,
            side_tab: SideTab::Segments,
            cov: cov::CovState::default(),
            marker_filter: String::new(),
            hoist: None,
            indent_all: false,
            edited: BTreeSet::new(),
            recovery_offer: vec![],
            recovery_written: String::new(),
            recovery_checked: 0.0,
            dirty_from: HashMap::new(),
            segments_fp: 0,
            saved_end: HashMap::new(),
            roll: roll::RollState::default(),
            message: None,
            speed: 3,
            end_mode: EndMode::Next,
            stop_on_changed: false,
            sticky_tip: None,
            route: vec![],
            route_target: None,
            node_dialog: None,
            renaming: None,
            delete_dialog: None,
            move_dialog: None,
            dup_dialog: None,
            save_dialog: None,
            discard_dialog: None,
            quit_ok: false,
            save_after_clean: false,
            join_dialog: None,
            chain_dialog: None,
            undo: vec![],
            redo: vec![],
            clip: None,
            ctx_copy: None,
            show_console: false,
            console_filter: String::new(),
            console_levels: [true; 3],
            console_seen: 0,
            frame_dialog: None,
            hydrating: None,
            gui_mark: None,
            stored_ids: HashSet::new(),
            max_memory_mb: 512,
            return_to_edit: true,
            edit_from: None,
            edit_at: None,
            mcp_on: false,
            show_drafts: true,
            drafts_fp: 0,
            agent_polled: 0.0,
            agent_sessions: vec![],
            taken_over: HashSet::new(),
            follow: None,
            share_view: false,
            shared: None,
            recording: false,
            rec_cfg: record::Settings::default(),
            bindings: record::Bindings::default(),
            rec_pass: false,
            rec_live: false,
            rec_hold: None,
            rec_resume: false,
            mapping: None,
            compare: None,
            checker: Default::default(),
            compare_run: true,
            compare_need_sync: false,
            compare_problem: None,
            compare_texture: None,
            compare_split: 0.5,
            compare_dialog: None,
            check_parity: false,
            compare_failed: false,
            parity: None,
            map_view: cov::MapView::Combined,
            rerun: None,
            seeking: false,
            base_next: None,
            reshow: false,
        }
    }
}

const KEEP_MENU_OPEN: &str = "dj_keep_menu_open";

/// A menu checkbox: clicking it leaves the menu open (see `menu_bar`).
trait KeepMenuOpen {
    fn keep_open(self) -> Self;
}

impl KeepMenuOpen for egui::Response {
    fn keep_open(self) -> Self {
        if self.clicked() {
            self.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(KEEP_MENU_OPEN), true));
        }
        self
    }
}

fn status_color(s: Status) -> Color32 {
    match s {
        Status::Clean => Color32::from_rgb(92, 184, 92),
        Status::Dirty => Color32::from_rgb(240, 160, 60),
    }
}

fn col_title(col: Option<&Collection>, id: &str) -> String {
    col.and_then(|c| c.replays.get(id)).map_or(String::new(), |r| r.title().to_string())
}

fn status_dot(ui: &mut egui::Ui, s: Status) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, status_color(s));
    resp
}

/// A small icon button in the style of VS Code's tree toolbar: two stacked boxes
/// with a minus (collapse all) or a plus (expand all).
fn fold_button(ui: &mut egui::Ui, collapse: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let c = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let stroke = egui::Stroke::new(1.2, c);
    let back = egui::Rect::from_min_size(rect.min + egui::vec2(7.0, 4.0), egui::vec2(9.0, 9.0));
    let front = egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 7.0), egui::vec2(9.0, 9.0));
    // The back box shows only as an L above and to the right of the front one.
    p.line_segment([back.left_top(), back.right_top()], stroke);
    p.line_segment([back.right_top(), back.right_bottom()], stroke);
    p.rect_filled(front, 1.0, ui.visuals().panel_fill);
    p.rect_stroke(front, 1.0, stroke, egui::StrokeKind::Inside);
    let m = front.center();
    p.line_segment([m - egui::vec2(2.5, 0.0), m + egui::vec2(2.5, 0.0)], stroke);
    if !collapse {
        p.line_segment([m - egui::vec2(0.0, 2.5), m + egui::vec2(0.0, 2.5)], stroke);
    }
    resp
}

/// `id` and every segment below it.
/// One named marker in the collection, for the sidebar's marker list.
struct ListedMarker<'a> {
    id: &'a str,
    frame: u64,
    glyph: &'static str,
    color: Color32,
    text: String,
    event: bool,
}

/// The sidebar's Markers tab: every event (then writes, resets, device changes and RNG
/// lines) in the collection, or only in `scope` and below it (the tree's Focus here),
/// alphabetically, with a filter on the marker and segment text.
/// Returns the segment and frame to show when a row is clicked.
fn marker_list(ui: &mut egui::Ui, col: &Collection, filter: &mut String, at: Option<(&str, u64)>, scope: Option<&str>) -> Option<(String, u64)> {
    let mut all: Vec<ListedMarker> = vec![];
    let inside: Option<HashSet<&str>> = scope.map(|h| subtree_ids(col, h).into_iter().collect());
    for (id, r) in &col.replays {
        if inside.as_ref().is_some_and(|s| !s.contains(id.as_str())) {
            continue;
        }
        for l in &r.lines {
            let (glyph, color, text, event) = match &l.command {
                Command::Event { text, .. } => ("⚑", Color32::from_rgb(240, 210, 90), text.clone(), true),
                Command::Write { region, address, bytes } => {
                    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                    ("✏", Color32::from_rgb(200, 120, 220), format!("{region} 0x{address:X} = {}", hex.join(" ")), false)
                }
                Command::Reset => ("⟲", Color32::from_rgb(230, 90, 80), "Reset".to_string(), false),
                Command::Device { port, device } => ("◆", Color32::from_rgb(120, 200, 230), format!("P{} device {device}", port + 1), false),
                Command::Rng { name, count } => ("🎲", Color32::from_gray(200), format!("RNG {name} {count}"), false),
                _ => continue,
            };
            all.push(ListedMarker { id, frame: l.frame, glyph, color, text, event });
        }
    }
    all.sort_by_cached_key(|m| (!m.event, m.text.to_lowercase(), col.replays[m.id].title().to_lowercase(), m.frame));
    let mut pick = None;
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if !filter.is_empty() && ui.small_button("✖").on_hover_text("Clear the filter").clicked() {
            filter.clear();
        }
        let w = ui.available_width();
        ui.add(egui::TextEdit::singleline(filter).id_salt("marker_filter").hint_text("Filter: marker or segment").desired_width(w));
    });
    let needle = filter.trim().to_lowercase();
    let shown: Vec<&ListedMarker> = all
        .iter()
        .filter(|m| needle.is_empty() || m.text.to_lowercase().contains(&needle) || col.replays[m.id].title().to_lowercase().contains(&needle))
        .collect();
    let what = if needle.is_empty() { "markers" } else { "matching markers" };
    let place = match scope {
        Some(h) => format!(" in the focused branch, \"{}\" and below", col_title(Some(col), h)),
        None => String::new(),
    };
    ui.label(RichText::new(format!("{} {what}{place}", shown.len())).weak().small());
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut was_event = true;
        for m in shown {
            if was_event && !m.event {
                ui.add_space(4.0);
                ui.label(RichText::new("Writes and other lines").weak().small());
                was_event = false;
            }
            let title = col.replays[m.id].title();
            let here = at == Some((m.id, m.frame));
            let path: Vec<&str> = col.chain(m.id).unwrap_or_default().iter().map(|i| col.replays[*i].title()).collect();
            // Second line: the segment, with its parent in front when another segment has the same title.
            let twin = col.replays.iter().any(|(k, r)| k != m.id && r.title() == title);
            let place = match (twin, path.len()) {
                (true, n) if n >= 2 => format!("frame {} · {} › {title}", m.frame, path[n - 2]),
                _ => format!("frame {} · {title}", m.frame),
            };
            let resp = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let r = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(m.glyph).color(m.color));
                            ui.add(egui::Button::selectable(here, RichText::new(&m.text).color(if m.event { Color32::from_gray(225) } else { m.color })).truncate())
                        })
                        .inner;
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.add(egui::Label::new(RichText::new(place).weak().small()).truncate());
                    });
                    r
                })
                .inner;
            ui.add_space(3.0);
            if resp.on_hover_text(format!("{}\nframe {} of \"{title}\"\nclick to show it", path.join(" › "), m.frame)).clicked() {
                pick = Some((m.id.to_string(), m.frame));
            }
        }
    });
    pick
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

/// `id` and every segment below it, borrowed from the collection.
fn subtree_ids<'a>(col: &'a Collection, id: &str) -> Vec<&'a str> {
    let Some((k, _)) = col.replays.get_key_value(id) else { return vec![] };
    let mut out = vec![k.as_str()];
    let mut i = 0;
    while i < out.len() {
        let kids = col.children(Some(out[i]));
        out.extend(kids);
        i += 1;
    }
    out
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

/// The first frame where two versions of a segment's lines differ, if they do.
fn first_change(a: &decomp_dj::format::Replay, b: &decomp_dj::format::Replay) -> Option<u64> {
    let n = a.lines.len().min(b.lines.len());
    for i in 0..n {
        let (x, y) = (&a.lines[i], &b.lines[i]);
        if x.frame != y.frame || x.command != y.command {
            return Some(x.frame.min(y.frame));
        }
    }
    match (a.lines.get(n), b.lines.get(n)) {
        (Some(l), _) | (_, Some(l)) => Some(l.frame),
        _ => None,
    }
}

/// Toggle for indenting every tree level: three bars stepping down to the right, drawn
/// on a highlighted background while every level is indented.
fn nest_button(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 3.0, ui.visuals().selection.bg_fill.gamma_multiply(0.6));
    } else if resp.hovered() {
        p.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let c = if resp.hovered() || on { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
    let stroke = egui::Stroke::new(1.2, c);
    let o = rect.min;
    // A tree guide: a vertical line with elbows into three rows, each one step further in.
    for (i, y) in [5.0, 10.0, 15.0].into_iter().enumerate() {
        let x = 4.0 + i as f32 * 3.5;
        p.line_segment([o + egui::vec2(x + 3.0, y), o + egui::vec2(16.5, y)], stroke);
        if i > 0 {
            let px = 4.0 + (i - 1) as f32 * 3.5;
            p.line_segment([o + egui::vec2(px + 1.0, y - 5.0 + 1.5), o + egui::vec2(px + 1.0, y)], stroke);
            p.line_segment([o + egui::vec2(px + 1.0, y), o + egui::vec2(x + 2.0, y)], stroke);
        } else {
            p.line_segment([o + egui::vec2(x, y), o + egui::vec2(x + 2.0, y)], stroke);
        }
    }
    resp
}

/// The list of children in the child button's menu (scrolls when long). The sticky child is marked.
fn child_list(ui: &mut egui::Ui, kids: &[(String, String)], sticky: Option<&String>, pick: &mut Option<String>) {
    ui.label(RichText::new(if kids.len() == 1 { "1 child segment".to_string() } else { format!("{} child segments", kids.len()) }).weak().small());
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        for (k, t) in kids {
            let is_sticky = sticky == Some(k);
            let text = if is_sticky { RichText::new(format!("{t}  (where you were)")).strong() } else { RichText::new(t.as_str()) };
            if ui.add(egui::Button::selectable(false, text).truncate()).on_hover_text(&k[..8.min(k.len())]).clicked() {
                *pick = Some(k.clone());
                ui.close();
            }
        }
    });
}

/// Lines the marker buttons stop at: the same kinds as the marker list.
fn is_marker(c: &Command) -> bool {
    matches!(c, Command::Event { .. } | Command::Write { .. } | Command::Device { .. } | Command::Reset)
}

fn icon_color(ui: &egui::Ui, r: &egui::Response) -> Color32 {
    if !r.enabled() {
        ui.visuals().widgets.noninteractive.fg_stroke.color.gamma_multiply(0.5)
    } else if r.hovered() {
        ui.visuals().widgets.hovered.fg_stroke.color
    } else {
        ui.visuals().widgets.inactive.fg_stroke.color
    }
}

/// Transport icon: one triangle and a mark beside it. Backward is the triangle with
/// the mark on its right (◀▌), forward the mirror (▌▶). `dot` draws a dot as the mark
/// (markers) instead of a bar (segments).
fn paint_skip(ui: &egui::Ui, r: &egui::Response, forward: bool, dot: bool) {
    let c = icon_color(ui, r);
    let p = ui.painter();
    let m = r.rect.center();
    let dir = if forward { 1.0 } else { -1.0 };
    // Triangle centred a little toward the direction of travel, mark on the other side.
    let tx = m.x + 2.5 * dir;
    let (h, w) = (5.0, 7.0);
    p.add(egui::Shape::convex_polygon(
        vec![egui::pos2(tx - w / 2.0 * dir, m.y - h), egui::pos2(tx + w / 2.0 * dir, m.y), egui::pos2(tx - w / 2.0 * dir, m.y + h)],
        c,
        egui::Stroke::NONE,
    ));
    let mx = m.x - 4.5 * dir;
    if dot {
        p.circle_filled(egui::pos2(mx, m.y), 2.4, c);
    } else {
        p.rect_filled(egui::Rect::from_center_size(egui::pos2(mx, m.y), egui::vec2(2.4, 2.0 * h)), 0.0, c);
    }
}

impl App {
    fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        decomp_dj::console::push(decomp_dj::console::Level::Info, "dj-gui", &text);
        self.message = Some((text, false));
    }

    fn error(&mut self, e: impl std::fmt::Display) {
        let text = format!("{e:#}");
        decomp_dj::console::push(decomp_dj::console::Level::Error, "dj-gui", &text);
        self.message = Some((text, true));
    }

    /// The console panel: the log of what dj, the cores, the workers and the GUI have said
    /// (`decomp_dj::console`), newest at the bottom.
    fn console_panel(&mut self, ui: &mut egui::Ui) {
        use decomp_dj::console::{self, Level};
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Debug console").strong());
            ui.add(egui::TextEdit::singleline(&mut self.console_filter).hint_text("Filter").desired_width(80.0));
            for (i, (name, tip)) in [("Info", "Notes and messages"), ("Warn", "Warnings"), ("Error", "Errors")].iter().enumerate() {
                ui.toggle_value(&mut self.console_levels[i], RichText::new(*name).small()).on_hover_text(format!("Show {}", tip.to_lowercase()));
            }
            if ui.small_button("Copy").on_hover_text("Copy the lines shown, for a bug report").clicked() {
                self.ctx_copy = Some(self.console_lines().iter().map(|l| format!("{:.1} {} {}", l.at, l.source, l.text)).collect::<Vec<_>>().join("\n"));
            }
            if ui.small_button("Clear").clicked() {
                console::clear();
            }
        });
        let lines = self.console_lines();
        self.console_seen = console::count();
        // The tree's text size and colour, in monospace so the columns line up.
        let size = ui.style().text_styles.get(&egui::TextStyle::Body).map_or(13.0, |f| f.size);
        let row = ui.fonts_mut(|f| f.row_height(&egui::FontId::monospace(size))) + ui.spacing().item_spacing.y;
        egui::ScrollArea::both().id_salt("console_lines").auto_shrink([false, false]).max_height(ui.available_height()).stick_to_bottom(true).show_rows(ui, row, lines.len(), |ui, range| {
            for l in &lines[range] {
                let colour = match l.level {
                    Level::Error => Color32::from_rgb(235, 100, 90),
                    Level::Warn => Color32::from_rgb(225, 185, 80),
                    Level::Info => ui.visuals().text_color(),
                };
                ui.label(RichText::new(format!("{:>7.1} {:<7} {}", l.at, l.source, l.text)).family(egui::FontFamily::Monospace).size(size).color(colour)).on_hover_text(&l.text);
            }
        });
    }

    /// The console's lines that pass its level toggles and filter.
    fn console_lines(&self) -> Vec<decomp_dj::console::Line> {
        use decomp_dj::console::Level;
        let filter = self.console_filter.to_lowercase();
        decomp_dj::console::lines()
            .into_iter()
            .filter(|l| self.console_levels[match l.level { Level::Info => 0, Level::Warn => 1, Level::Error => 2 }])
            .filter(|l| filter.is_empty() || l.text.to_lowercase().contains(&filter) || l.source.to_lowercase().contains(&filter))
            .collect()
    }

    // ---- collection and core ----

    fn open_collection(&mut self, dir: &Path) {
        match Collection::open(dir) {
            Ok(mut col) => {
                self.mcp_on = col.settings().ok().and_then(|l| l.mcp).and_then(|m| m.gui).unwrap_or(false);
                self.agent_sessions.clear();
                self.share_view = false;
                self.shared = None;
                if self.mcp_on && self.show_drafts {
                    let _ = col.load_drafts();
                }
                self.drafts_fp = agent::drafts_fingerprint(dir);
                self.segments_fp = agent::segments_fingerprint(dir);
                self.dirty_from.clear();
                self.follow = None;
                // Only one libretro core can be loaded per process, so drop the old one first.
                self.session = None;
                self.unload_compare();
                { self.end_pass(); self.pb = None; }
                self.playing = false;
                self.texture = None;
                self.edited.clear();
                self.selected = None;
                self.hoist = None;
                self.undo.clear();
                self.redo.clear();
                self.roll.forget();
                let first = col.tree_order().first().map(|(id, _)| id.to_string());
                let n = col.replays.len();
                self.max_memory_mb = col.settings().ok().and_then(|l| l.max_memory_mb).unwrap_or(512);
                let local = col.settings().unwrap_or_default();
                self.bindings = record::Bindings::from_local(local.keyboard.as_ref());
                self.rec_cfg = record::Settings::from_local(local.record.as_ref());
                self.set_recording(false);
                self.hydrating = None;
                self.saved_end = col.replays.iter().map(|(id, r)| (id.clone(), ending_sig(r))).collect();
                self.gui_mark = Some(decomp_dj::cache::GuiMark::new(&col.root));
                // The project being left closes normally: nothing to recover from it.
                if let Some(old) = &self.col {
                    decomp_dj::recovery::clear(&old.root);
                }
                self.recovery_written.clear();
                // A crash or hang in a core leaves a note, and unsaved edits are kept as they're
                // made; what a dj-gui that's gone left is offered back.
                decomp_dj::crash::set_note_path(&col.root);
                decomp_dj::crash::start_watchdog(std::time::Duration::from_secs(10));
                self.recovery_offer = decomp_dj::recovery::left_behind(&col.root);
                self.col = Some(col);
                self.open_session();
                self.refresh_stored();
                self.info(format!("Opened {} ({n} segments)", dir.display()));
                let warnings = self.col.as_ref().map(|c| c.warnings.clone()).unwrap_or_default();
                for w in warnings {
                    self.error(format!("Note: {w}"));
                }
                if let Some(id) = first {
                    self.select(&id);
                }
            }
            Err(e) => self.error(format!("Could not open {}: {e:#}", dir.display())),
        }
    }

    fn open_session(&mut self) {
        let Some(col) = &self.col else { return };
        self.session = None;
        let local = match col.settings() {
            Ok(l) => l,
            Err(e) => {
                self.core_error = Some(format!("{e:#}"));
                return;
            }
        };
        let (rom, core) = match (&local.rom, &local.core) {
            // Already resolved against the project folder by `settings`.
            (Some(r), Some(c)) => (r.clone(), c.clone()),
            (None, _) => {
                self.core_error = Some("No ROM set for this project on this machine. Use Emulation > Set ROM…".into());
                return;
            }
            (_, None) => {
                self.core_error = Some("No libretro core set for this project on this machine. Use Emulation > Set core…".into());
                return;
            }
        };
        match Session::open(col, &rom, &core) {
            Ok(s) => {
                self.base_next = None;
                self.compare_need_sync = true;
                let (fps, aspect) = s.core.timing();
                self.fps = if fps > 1.0 { fps } else { 60.0 };
                self.aspect = aspect;
                self.session = Some(s);
                self.core_error = None;
            }
            Err(e) => self.core_error = Some(format!("Could not start the core: {e:#}")),
        }
    }

    fn set_local(&mut self, rom: Option<PathBuf>, core: Option<PathBuf>) {
        let Some(col) = &self.col else { return };
        let mut local: LocalConfig = col.load_local().unwrap_or_default();
        if rom.is_some() {
            local.rom = rom;
        }
        if core.is_some() {
            local.core = core;
        }
        if let Err(e) = col.save_local(&local) {
            self.error(e);
            return;
        }
        { self.end_pass(); self.pb = None; }
        self.open_session();
        if let Some(id) = self.selected.clone() {
            self.select(&id);
        }
    }

    /// Segments with changes not saved yet, and drafts (unsaved segments an agent wrote).
    fn unsaved(&self) -> BTreeSet<String> {
        let mut out = self.edited.clone();
        if let Some(col) = &self.col {
            out.extend(col.drafts.iter().filter(|d| col.replays.contains_key(*d)).cloned());
        }
        // A draft an agent session has open is the agent's work, not the person's.
        out.retain(|id| !self.agent_owned(id));
        out
    }

    /// Edits that Open, Reload or Quit would lose: everything unsaved except drafts nobody
    /// edited here, which stay in `.local/drafts/` and come back on the next open.
    fn unsaved_edits(&self) -> usize {
        self.edited.iter().filter(|id| !self.agent_owned(id)).count()
    }

    /// Keeps the unsaved edits in `.local/recovery/` (when they've changed), so a crash or a
    /// hang in a core doesn't lose them.
    fn write_recovery(&mut self) {
        let Some(col) = &self.col else { return };
        let segments: BTreeMap<String, Option<String>> = self.edited.iter().filter(|id| !self.agent_owned(id)).map(|id| (id.clone(), col.replays.get(id).map(|r| r.to_text()))).collect();
        let sig = serde_json::to_string(&segments).unwrap_or_default();
        if sig == self.recovery_written {
            return;
        }
        match decomp_dj::recovery::write(&col.root, segments) {
            Ok(()) => self.recovery_written = sig,
            Err(e) => self.error(format!("Couldn't keep a copy of the unsaved edits: {e:#}")),
        }
    }

    /// Offers back what a dj-gui that's gone left: its unsaved edits, as an unsaved change
    /// (Undo takes it back), and its crash note.
    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(left) = self.recovery_offer.first().cloned() else { return };
        let n = left.snapshot.as_ref().map_or(0, |s| s.segments.len());
        let (mut restore, mut discard) = (false, false);
        egui::Window::new("dj-gui didn't close normally").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            if let Some(note) = &left.note {
                ui.label(RichText::new(note.lines().next().unwrap_or("")).strong());
                for l in note.lines().skip(1) {
                    ui.label(RichText::new(l).monospace().small());
                }
                ui.add_space(6.0);
            } else {
                ui.label(RichText::new("The last dj-gui to open this project ended without closing it (it crashed, or was quit by force).").strong());
            }
            if n > 0 {
                ui.label(format!("It had unsaved edits in {n} segment{}. Restore them? They come back as unsaved edits (Undo takes them back); Discard drops them for good.", if n == 1 { "" } else { "s" }));
                ui.horizontal(|ui| {
                    restore = ui.button("Restore").clicked();
                    discard = ui.button("Discard").clicked();
                });
            } else {
                discard = ui.button("OK").clicked();
            }
        });
        if !(restore || discard) {
            return;
        }
        self.recovery_offer.remove(0);
        let Some(root) = self.col.as_ref().map(|c| c.root.clone()) else { return };
        if restore {
            let mut segments = vec![];
            for (id, text) in left.snapshot.map(|s| s.segments).unwrap_or_default() {
                match text.map(|t| decomp_dj::format::Replay::parse(&t)).transpose() {
                    Ok(r) => segments.push((id, r)),
                    Err(e) => {
                        self.error(format!("Couldn't restore segment {id}: {e:#}"));
                        return;
                    }
                }
            }
            let step = UndoStep { label: "Restore unsaved edits".to_string(), segments, selected: self.selected.clone(), frame: self.playhead() };
            let back = self.restore(step);
            self.undo.push(back);
            self.redo.clear();
            self.info(format!("Restored the unsaved edits of {n} segment{}. Undo takes them back.", if n == 1 { "" } else { "s" }));
        }
        decomp_dj::recovery::forget(&root, left.pid);
    }

    /// Runs `what` now when nothing would be lost; otherwise asks first.
    fn discard_guard(&mut self, what: Discard) {
        if self.unsaved_edits() == 0 {
            self.discard_go(what);
        } else {
            self.discard_dialog = Some(what);
        }
    }

    fn discard_go(&mut self, what: Discard) {
        match what {
            Discard::Open(dir) => self.open_collection(&dir),
            Discard::Reload => self.reload(),
            Discard::Quit => {
                self.quit_ok = true;
                if let Some(ctx) = CTX.with(|c| c.borrow().clone()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Segments that saving now would leave dirty: the unsaved ones and everything below them.
    fn dirty_on_save(&self) -> usize {
        let Some(col) = &self.col else { return 0 };
        let mut dirty = BTreeSet::new();
        for id in self.unsaved() {
            if col.replays.contains_key(&id) {
                dirty.extend(subtree(col, &id).into_iter().filter(|i| col.tree_status(i) == Status::Dirty && !self.agent_owned(i)));
            }
        }
        dirty.len()
    }

    /// Save, but first offer to clean when the save would leave segments dirty: the unsaved
    /// ones and everything below them (docs/playback.md, "Clean and dirty").
    fn save(&mut self) {
        match self.dirty_on_save() {
            0 => self.save_now(),
            n => self.save_dialog = Some(SaveAsk::Dirty(n)),
        }
    }

    /// After "Clean, then save": saves when nothing needs a look, otherwise asks once more,
    /// so changed endings are never saved unseen.
    fn after_clean(&mut self, changed: usize, failed: Option<String>) {
        let dirty = self.dirty_on_save();
        if changed == 0 && dirty == 0 && failed.is_none() {
            self.save_now();
        } else {
            self.save_dialog = Some(SaveAsk::AfterClean { changed, dirty, failed });
        }
    }

    /// Saves every unsaved change. Drafts move into segments/ with any edits made here,
    /// except drafts an agent still has open in a session (and drafts below those).
    fn save_now(&mut self) {
        // Read the sessions again now rather than trust the last poll: a session opened on a
        // draft in the last second must not see it moved under it.
        if self.mcp_on {
            if let Some(root) = self.col.as_ref().map(|c| c.root.clone()) {
                self.agent_sessions = agent::sessions(&root);
            }
        }
        let in_session: HashSet<String> = self.agent_sessions.iter().map(|s| s.draft.clone()).filter(|d| !self.taken_over.contains(d)).collect();
        let Some(col) = &mut self.col else { return };
        let ids: Vec<String> = self.edited.iter().cloned().collect();
        let mut saved = vec![];
        for id in &ids {
            // Drafts are moved below, once their parents are saved.
            if col.is_draft(id) && col.replays.contains_key(id) {
                continue;
            }
            // A segment removed (deleted, or by undo of a split or a new segment) loses its file.
            let res = if col.replays.contains_key(id) { col.save_replay(id) } else { col.delete_replay_file(id) };
            if let Err(e) = res {
                self.message = Some((format!("Saving {id}: {e:#}"), true));
                return;
            }
            if col.is_draft(id) {
                col.drafts.remove(id);
            }
            saved.push(id.clone());
        }
        let drafts: Vec<String> = col.tree_order().into_iter().map(|(id, _)| id.to_string()).filter(|id| col.is_draft(id)).collect();
        let mut held = 0;
        for id in drafts {
            let parent_held = col.replays[&id].parent().is_some_and(|p| col.is_draft(p));
            if in_session.contains(&id) || parent_held {
                held += 1;
                continue;
            }
            if let Err(e) = col.draft_to_segment(&id) {
                self.message = Some((format!("Saving draft {id}: {e:#}"), true));
                return;
            }
            saved.push(id);
        }
        for id in &saved {
            self.edited.remove(id);
        }
        self.drafts_fp = agent::drafts_fingerprint(&col.root);
        self.segments_fp = agent::segments_fingerprint(&col.root);
        self.note_saved(&saved);
        self.coverage_stale();
        let n = saved.len();
        if held > 0 {
            self.info(format!("Saved {n} segment(s). {held} draft(s) still open in an agent session stay drafts."));
        } else {
            self.info(format!("Saved {n} segment(s)"));
        }
    }

    /// Why the segment `id` can't be saved on its own, if it can't.
    fn save_one_blocked(&self, id: &str) -> Option<&'static str> {
        let col = self.col.as_ref()?;
        if !col.replays.contains_key(id) || !self.unsaved().contains(id) {
            return Some("No unsaved changes in this segment");
        }
        if self.agent_owned(id) {
            return Some("An agent session has this draft open");
        }
        let parent = col.replays[id].parent()?;
        let parent_on_disk = col.paths.get(parent).is_some_and(|p| p.exists()) && !col.is_draft(parent);
        (!parent_on_disk).then_some("Its parent has never been saved: save that one first (or everything)")
    }

    /// Saves only `id`: its file, or a draft moved into segments/. Other edits stay unsaved.
    fn save_one(&mut self, id: &str) {
        if let Some(why) = self.save_one_blocked(id) {
            self.error(why);
            return;
        }
        let Some(col) = &mut self.col else { return };
        let res = if col.is_draft(id) { col.draft_to_segment(id) } else { col.save_replay(id) };
        if let Err(e) = res {
            self.error(format!("Saving {id}: {e:#}"));
            return;
        }
        self.edited.remove(id);
        self.drafts_fp = agent::drafts_fingerprint(&col.root);
        self.segments_fp = agent::segments_fingerprint(&col.root);
        let dirty = col.tree_status(id) == Status::Dirty;
        self.note_saved(&[id.to_string()]);
        self.coverage_stale();
        let title = col_title(self.col.as_ref(), id);
        let left = self.unsaved().len();
        let mut msg = format!("Saved \"{title}\".");
        if dirty {
            msg.push_str(" It is dirty: play it to its end (or clean it) and save again to keep its hashes.");
        }
        if left > 0 {
            msg.push_str(&format!(" {left} other unsaved change{} left.", if left == 1 { "" } else { "s" }));
        }
        self.info(msg);
    }

    /// Why the segment `id` can't be reverted to its file, if it can't.
    fn revert_blocked(&self, id: &str) -> Option<&'static str> {
        let col = self.col.as_ref()?;
        if !self.edited.contains(id) || !col.replays.contains_key(id) {
            return Some("No unsaved changes in this segment");
        }
        if self.agent_owned(id) {
            return Some("An agent session has this draft open");
        }
        if !col.paths.get(id).is_some_and(|p| p.exists()) {
            return Some("Never saved, so there is no file to go back to (Delete removes it)");
        }
        None
    }

    /// Puts back the saved file of `id`, dropping its unsaved changes (one undo step brings
    /// them back). Other segments keep their edits.
    fn revert_one(&mut self, id: &str) {
        if let Some(why) = self.revert_blocked(id) {
            self.error(why);
            return;
        }
        let Some(path) = self.col.as_ref().and_then(|c| c.paths.get(id)).cloned() else { return };
        let disk = match decomp_dj::format::Replay::load(&path) {
            Ok(r) => r,
            Err(e) => {
                self.error(format!("{e:#}"));
                return;
            }
        };
        self.rec_pass = false;
        let step = UndoStep { label: "Revert".to_string(), segments: vec![(id.to_string(), Some(disk))], selected: self.selected.clone(), frame: self.playhead() };
        let back = self.restore(step);
        self.undo.push(back);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.edited.remove(id);
        let title = col_title(self.col.as_ref(), id);
        self.info(format!("Reverted \"{title}\" to its saved file. Undo brings the changes back."));
    }

    /// Records the end hashes `ids` have on disk now (just loaded or saved).
    fn note_saved(&mut self, ids: &[String]) {
        let Some(col) = &self.col else { return };
        for id in ids {
            match col.replays.get(id) {
                Some(r) => self.saved_end.insert(id.clone(), ending_sig(r)),
                None => self.saved_end.remove(id),
            };
        }
    }

    /// An unsaved segment whose end hash differs from the one in its saved file: shown with
    /// a "!" until it is saved (or the change is undone).
    fn ending_changed(&self, id: &str) -> bool {
        if !self.edited.contains(id) {
            return false;
        }
        let Some(Some(old)) = self.saved_end.get(id) else { return false };
        self.col.as_ref().and_then(|c| c.replays.get(id)).and_then(ending_sig).is_some_and(|now| &now != old)
    }

    fn reload(&mut self) {
        if let Some(root) = self.col.as_ref().map(|c| c.root.clone()) {
            let sel = self.selected.clone();
            self.open_collection(&root);
            if let Some(id) = sel {
                if self.col.as_ref().is_some_and(|c| c.replays.contains_key(&id)) {
                    self.select(&id);
                }
            }
        }
    }

    // ---- playback ----

    fn select(&mut self, id: &str) {
        if self.hydrating.is_some() {
            self.selected = Some(id.to_string());
            self.info("Cleaning: the segment is shown once the clean stops or finishes.");
            return;
        }
        self.playing = false;
        self.route.clear();
        self.route_target = None;
        self.job = None;
        self.selected = Some(id.to_string());
        { self.end_pass(); self.pb = None; }
        if let Err(e) = self.try_select(id) {
            self.error(format!("Could not reach the start of this segment: {e:#}"));
        }
    }

    fn try_select(&mut self, id: &str) -> Result<()> {
        let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
        let chain: Vec<String> = col.chain(id)?.into_iter().map(String::from).collect();
        let n = chain.len();
        let fast = n == 1 || s.load_cached_state(col, &chain[n - 2])?.is_some();
        if fast {
            let start_frame = s.enter_replay(col, id)?;
            self.begin_playback(id, start_frame)?;
            self.roll.follow_to(0);
            self.seek(0);
            return Ok(());
        }
        // No cached start state: play the ancestors a slice at a time, from the
        // deepest one that has a cached end state, or from power-on.
        let mut first = 0;
        for i in (0..n - 1).rev() {
            if let Some(state) = s.load_cached_state(col, &chain[i])? {
                s.load_state(&state)?;
                first = i + 1;
                break;
            }
        }
        if first == 0 {
            s.start_trunk(col, &col.replays[&chain[0]])?;
        }
        let frames = |c: &String| col.replays[c].frames().unwrap_or(0);
        let queue: Vec<String> = chain[first..n - 1].to_vec();
        let total = queue.iter().map(frames).sum();
        let start_frame = trunk_start_frame(&col.replays[&chain[0]]) + chain[..n - 1].iter().map(frames).sum::<u64>();
        self.job = Some(Job::Enter { id: id.to_string(), queue, qi: 0, frame: 0, start_frame, done: 0, total, then_seek: None });
        Ok(())
    }

    /// Re-reads which frames of the current content have a stored snapshot.
    fn refresh_pass_frames(&mut self) {
        let (Some(col), Some(s), Some(pb)) = (&self.col, &self.session, &mut self.pb) else { return };
        if let Some(start) = &pb.start_hash {
            pb.stored = s.stored_frames(col, &pb.id, start).unwrap_or_default();
        }
    }

    /// Ends the pass being recorded, if any, keeping the snapshots it took.
    fn end_pass(&mut self) {
        let (Some(s), Some(pb)) = (&mut self.session, &mut self.pb) else { return };
        if let Some(rec) = pb.rec.take() {
            if let Err(e) = s.keep_snapshots(rec) {
                self.error(format!("Could not store this pass's snapshots: {e:#}"));
            }
        }
    }

    /// The core is at frame 0 of `id`: set up playback there.
    fn begin_playback(&mut self, id: &str, start_frame: u64) -> Result<()> {
        self.end_pass();
        let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
        let r = &col.replays[id];
        let frames = r.frames().unwrap_or(0);
        let checkpoints = r.checkpoints().unwrap_or_default().into_iter().collect();
        let rec = start_pass(s, col, id, start_frame)?;
        let snap = s.snapshot()?;
        let start_hash = rec.start_hash.clone();
        // Keep the start state in the database too, so the next visit starts here directly.
        if let Some(c) = &mut s.cache {
            c.put_snapshot(&start_hash, &snap)?;
        }
        let stored = s.stored_frames(col, id, &start_hash).unwrap_or_default();
        self.pb = Some(Playback { id: id.to_string(), start_frame, frames, ran: 0, snapshots: BTreeMap::from([(0, snap)]), checkpoints, checkpoint_miss: None, checkpoints_seen: BTreeMap::new(), end_hash: None, start_hash: Some(start_hash), stored, stale_from: None, picture_stale: false, rec: Some(rec) });
        self.base_next = None;
        self.edit_from = None;
        self.coverage_restart_live();
        Ok(())
    }

    /// Does one slice of the current job.
    fn work(&mut self) {
        if self.hydrating.is_some() {
            self.hydrate_work();
            return;
        }
        let Some(job) = self.job.take() else { return };
        if let Err(e) = self.try_work(job) {
            self.job = None;
            self.playing = false;
            self.error(e);
        }
    }

    fn try_work(&mut self, job: Job) -> Result<()> {
        let t0 = std::time::Instant::now();
        match job {
            Job::Seek { from, to } => {
                self.seeking = true;
                let r = self.seek_slice(from, to, t0);
                self.seeking = false;
                r
            }
            Job::Enter { id, queue, mut qi, mut frame, start_frame, mut done, total, then_seek } => {
                loop {
                    let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
                    if qi == queue.len() {
                        self.begin_playback(&id, start_frame)?;
                        self.roll.follow_to(then_seek.unwrap_or(0));
                        self.seek(then_seek.unwrap_or(0));
                        return Ok(());
                    }
                    let q = &queue[qi];
                    let qframes = col.replays[q].frames().unwrap_or(0);
                    if frame < qframes {
                        if frame == 0 {
                            s.restore_inputs(col, q, 0)?;
                        }
                        s.step(col, q, frame, false)?;
                        frame += 1;
                        done += 1;
                    }
                    if frame >= qframes {
                        qi += 1;
                        frame = 0;
                    }
                    if t0.elapsed() > JOB_SLICE {
                        self.job = Some(Job::Enter { id, queue, qi, frame, start_frame, done, total, then_seek });
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Runs a far seek's frames for one slice of time.
    fn seek_slice(&mut self, from: u64, to: u64, t0: std::time::Instant) -> Result<()> {
        loop {
            let ran = self.pb.as_ref().map_or(to, |p| p.ran);
            if ran >= to {
                return Ok(());
            }
            let last = ran + 1 == to;
            self.run_one(last)?;
            if !last && t0.elapsed() > JOB_SLICE {
                self.job = Some(Job::Seek { from, to });
                return Ok(());
            }
        }
    }

    /// Progress of the current job, for the status bar.
    fn job_progress(&self) -> Option<(f32, String)> {
        match self.job.as_ref()? {
            Job::Seek { from, to } => {
                let ran = self.pb.as_ref().map_or(*from, |p| p.ran);
                let f = (ran.saturating_sub(*from)) as f32 / (to - from).max(1) as f32;
                Some((f, format!("Seeking to frame {}", to.saturating_sub(1))))
            }
            Job::Enter { id, done, total, .. } => {
                let title = col_title(self.col.as_ref(), id);
                Some((*done as f32 / (*total).max(1) as f32, format!("Playing up to \"{title}\"")))
            }
        }
    }

    /// Runs one frame of the selected replay.
    fn run_one(&mut self, capture: bool) -> Result<()> {
        let Some(pb) = &self.pb else { return Ok(()) };
        if pb.ran >= pb.frames || self.col.is_none() || self.session.is_none() {
            return Ok(());
        }
        let (id, k) = (pb.id.clone(), pb.ran);
        // Playing on after a failure: sync first, as after a jump.
        if self.compare_failed {
            self.compare_failed = false;
            self.compare_problem = None;
            self.checker.lose_step();
            self.compare_need_sync = true;
        }
        // While checking parity, every frame is drawn, so a failure shows its own frame.
        let capture = capture || (self.check_parity && self.compare_run && self.compare.is_some() && !self.seeking);
        // Anything but the frame after the last one run is a jump: the comparison core
        // falls out of step and syncs again before the next frame shown.
        if self.base_next.as_ref().is_none_or(|(i, f)| *i != id || *f != k) {
            self.checker.lose_step();
            self.compare_need_sync = true;
        }
        if capture && self.compare_need_sync && self.compare_run && self.compare.is_some() && !egui_ctx().input(|i| i.pointer.any_down()) {
            // Not while a button is held (dragging the ruler): that syncs once it's let go.
            self.compare_sync(&id, k);
        }
        let (Some(col), Some(s), Some(pb)) = (&self.col, &mut self.session, &mut self.pb) else { return Ok(()) };
        // The pass records only while it runs straight on.
        if pb.rec.as_ref().is_some_and(|r| r.f != k) {
            s.keep_snapshots(pb.rec.take().unwrap())?;
        }
        let want_snap = k % SNAP_EVERY == 0 && !pb.snapshots.contains_key(&k);
        let from_pass = pb.rec.as_ref().is_some_and(|r| r.stores_at(k));
        if want_snap && !from_pass {
            pb.snapshots.insert(k, s.snapshot()?);
            trim_snapshots(&mut pb.snapshots, k, self.max_memory_mb * 1024 * 1024);
        }
        if let Some(want) = pb.checkpoints.get(&k) {
            let got = s.state_hash(col)?;
            if &got != want && pb.checkpoint_miss.is_none_or(|m| k < m) {
                pb.checkpoint_miss = Some(k);
            }
            pb.checkpoints_seen.insert(k, got);
        }
        let show = capture || k + 1 == pb.frames;
        let frame = match &mut pb.rec {
            Some(rec) => {
                s.advance(col, rec, show)?;
                if want_snap && from_pass {
                    if let Some(st) = rec.snapshot_at(k) {
                        pb.snapshots.insert(k, st.to_vec());
                        trim_snapshots(&mut pb.snapshots, k, self.max_memory_mb * 1024 * 1024);
                    }
                }
                rec.last_frame.take()
            }
            None => s.step(col, &pb.id, k, show)?,
        };
        pb.ran += 1;
        self.base_next = Some((id.clone(), k + 1));
        // A failure stops playback of both cores (Some: why). `passed`: the frame was checked
        // and matched.
        let mut failed: Option<(String, decomp_dj::compare::Diffs)> = None;
        let mut passed = false;
        if self.compare_run {
            if let Some(c) = &mut self.compare {
                if let Some(st) = self.checker.step(c, &mut s.core, col, &id, k, pb.start_frame + k, capture, self.check_parity) {
                    if let Some(f) = st.picture {
                        let img = egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
                        match &mut self.compare_texture {
                            Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                            None => self.compare_texture = Some(egui_ctx().load_texture("compare", img, egui::TextureOptions::NEAREST)),
                        }
                    }
                    match st.checked {
                        Checked::Passed => passed = true,
                        // A [WRITE] it couldn't follow: sync again at the next frame shown
                        // (this frame isn't checked: the core didn't see the write).
                        Checked::NeedsSync => self.compare_need_sync = true,
                        Checked::Failed(msg) => failed = Some((msg, st.diffs)),
                        Checked::Skipped(_) | Checked::Ran => {}
                    }
                }
            }
        }
        // While checking parity, each frame's coverage is read on its own: the code a frame
        // that passed or failed ran.
        let checking = self.check_parity && self.compare_run && self.compare.is_some();
        let frame_flags = if checking { decomp_dj::parity::take_code_flags(&mut s.core) } else { None };
        if let Some(fl) = &frame_flags {
            self.cov.absorb(fl, egui_ctx().input(|i| i.time));
            if passed && failed.is_none() {
                if let Some(p) = &mut self.parity {
                    p.add_pass(fl);
                }
            }
        }
        let checked = if failed.is_some() { Some(false) } else { passed.then_some(true) };
        if let Some((msg, diffs)) = failed {
            let synced = self.checker.synced.clone().unwrap_or((id.clone(), k));
            let from = if synced.0 == id { format!("frame {}", synced.1) } else { format!("frame {} of \"{}\"", synced.1, col_title(Some(col), &synced.0)) };
            if let (Some(p), Some(c)) = (&mut self.parity, &self.compare) {
                p.add_failure(self.checker.failure(c, &id, k, &msg, frame_flags.unwrap_or_default(), diffs.clone()), s.cache.as_ref());
            }
            self.checker.lose_step();
            self.compare_failed = true;
            let differed = if diffs.is_empty() { String::new() } else { format!("\n\nWhat differed:\n{}", diffs.lines(8).iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")) };
            self.compare_problem = Some(format!("{msg}{differed}\n\nSynced at {from}. Play or step on to sync again."));
            self.playing = false;
        }
        let reached_end = pb.ran == pb.frames;
        // Frames run since the edit are up to date again.
        if let Some(st) = pb.stale_from {
            pb.stale_from = (pb.ran < pb.frames).then_some(st.max(pb.ran));
        }
        if frame.is_some() {
            pb.picture_stale = false;
        }
        let mut recorded = false;
        if pb.ran == pb.frames {
            if let Some(rec) = pb.rec.take() {
                // A whole pass from frame 0: write it to the database (the end state too,
                // where every child starts).
                pb.end_hash = Some(s.finish(col, rec)?.end_hash);
                recorded = s.cache.is_some();
            } else {
                let h = s.state_hash(col)?;
                // The end state is where every child starts: keep it in the database.
                if !col.children(Some(&pb.id)).is_empty() {
                    let state = s.snapshot()?;
                    if let Some(c) = &mut s.cache {
                        c.put_snapshot(&h, &state)?;
                    }
                }
                pb.end_hash = Some(h);
            }
        }
        if let Some(f) = frame {
            let img = egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
            match &mut self.texture {
                Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                None => self.texture = Some(egui_ctx().load_texture("preview", img, egui::TextureOptions::NEAREST)),
            }
        }
        // Recording past the end grows the segment a frame at a time: its end isn't reached.
        if reached_end && !(self.recording && self.rec_cfg.extend) {
            self.auto_record();
        }
        if recorded {
            self.refresh_stored();
            self.refresh_pass_frames();
        }
        if self.rerun.is_some() {
            self.rerun_frame(&id, k, checked);
        }
        Ok(())
    }

    /// At the last frame: records the hashes this playback reached (`try_record`), when it
    /// started where its clean parent ends. An unsaved change, like an edit. A changed
    /// ending is recorded too, and flagged with "!" until saved.
    fn auto_record(&mut self) {
        // An agent may still be writing this draft.
        if self.pb.as_ref().is_some_and(|pb| self.agent_owned(&pb.id)) {
            return;
        }
        let (Some(col), Some(pb)) = (&mut self.col, &self.pb) else { return };
        let (Some(end), Some(start)) = (pb.end_hash.clone(), pb.start_hash.clone()) else { return };
        let id = pb.id.clone();
        let seen: Vec<(u64, String)> = pb.checkpoints_seen.iter().map(|(f, h)| (*f, h.clone())).collect();
        let before = col.replays[&id].clone();
        let verdict = decomp_dj::hydrate::try_record(col, &id, &start, pb.start_frame, &end, Some(&seen));
        if matches!(verdict, "recorded" | "cleaned" | "changed") {
            self.fold_into_undo(&id, before);
            let Some(col) = &self.col else { return };
            let title = col_title(Some(col), &id);
            let cps: BTreeMap<u64, String> = col.replays[&id].checkpoints().unwrap_or_default().into_iter().collect();
            if let Some(pb) = &mut self.pb {
                pb.checkpoints = cps;
                pb.checkpoint_miss = None;
            }
            self.edited.insert(id.clone());
            self.dirty_from.remove(&id);
            self.coverage_stale();
            self.info(match verdict {
                "recorded" => format!("Recorded the hashes of \"{title}\". Save to keep them."),
                "cleaned" => format!("\"{title}\" still ends the same: recorded its new start hash. Save to keep it."),
                _ => format!("\"{title}\" ends differently from its saved version: recorded the new ending (marked !). Save to keep it, or undo."),
            });
        }
    }

    /// Shows frame `target` (0-based) of the selected replay.
    fn seek(&mut self, target: u64) {
        if let Some(Job::Enter { then_seek, .. }) = &mut self.job {
            *then_seek = Some(target);
            return;
        }
        if matches!(self.job, Some(Job::Seek { .. })) {
            self.job = None;
        }
        if let Err(e) = self.try_seek(target) {
            self.playing = false;
            self.error(e);
        }
    }

    fn try_seek(&mut self, target: u64) -> Result<()> {
        let Some(pb) = &self.pb else { return Ok(()) };
        if pb.frames == 0 {
            return Ok(());
        }
        let want_ran = (target + 1).min(pb.frames);
        // A snapshot in the database nearer than any in memory: load it into memory first.
        let mem_k = pb.snapshots.range(..want_ran).next_back().map(|(k, _)| *k).unwrap_or(0);
        let db_k = pb.stored.iter().rev().find(|f| **f < want_ran).copied();
        if let (Some(f), Some(start)) = (db_k, pb.start_hash.clone()) {
            if f > mem_k && (want_ran <= pb.ran || f > pb.ran) {
                let id = pb.id.clone();
                if let (Some(col), Some(s)) = (&self.col, &self.session) {
                    if let Some((sf, state)) = s.nearest_stored(col, &id, &start, f)? {
                        self.pb.as_mut().unwrap().snapshots.insert(sf, state);
                    }
                }
            }
        }
        let Some(pb) = &self.pb else { return Ok(()) };
        if want_ran != pb.ran || self.texture.is_none() || std::mem::take(&mut self.reshow) {
            // Restart from the nearest snapshot that still leaves one frame to draw,
            // unless running on from where we are is quicker.
            let (&k, _) = pb.snapshots.range(..want_ran).next_back().ok_or_else(|| anyhow!("no snapshot"))?;
            if want_ran <= pb.ran || k > pb.ran {
                let id = pb.id.clone();
                let state = pb.snapshots[&k].clone();
                let (Some(col), Some(s)) = (&self.col, &mut self.session) else { return Ok(()) };
                s.load_state(&state)?;
                let pb = self.pb.as_mut().unwrap();
                if let Some(rec) = pb.rec.take() {
                    s.keep_snapshots(rec)?;
                }
                if k == 0 {
                    // Back at the start: record a new pass.
                    pb.rec = Some(start_pass(s, col, &id, pb.start_frame)?);
                } else {
                    s.restore_inputs(col, &id, k)?;
                }
                pb.ran = k;
                pb.end_hash = None;
            }
            let ran = self.pb.as_ref().unwrap().ran;
            if want_ran - ran > 30 {
                // Far ahead: run it in slices so the window stays responsive.
                self.job = Some(Job::Seek { from: ran, to: want_ran });
                self.work();
                return Ok(());
            }
            self.seeking = true;
            let mut r = Ok(());
            while self.pb.as_ref().unwrap().ran < want_ran && r.is_ok() {
                let last = self.pb.as_ref().unwrap().ran + 1 == want_ran;
                r = self.run_one(last);
            }
            self.seeking = false;
            r?;
        }
        Ok(())
    }

    fn step_by(&mut self, delta: i64) {
        self.playing = false;
        // Stepping past either end crosses into the neighbour the roll shows there.
        if let Some(pb) = &self.pb {
            let at = pb.shown();
            if delta < 0 && at.is_none_or(|f| f == 0) && self.job.is_none() {
                if let Some(p) = self.col.as_ref().and_then(|c| c.replays.get(&pb.id)).and_then(|r| r.parent()).map(String::from) {
                    self.segment_back(&p);
                    return;
                }
            }
            if delta > 0 && at.is_some_and(|f| f + 1 >= pb.frames) && self.job.is_none() {
                if let Some(n) = self.next_neighbour() {
                    self.segment_forward(&n);
                    return;
                }
            }
        }
        if let Some(cur) = self.pb.as_ref().and_then(|p| p.shown()) {
            let t = (cur as i64 + delta).max(0) as u64;
            self.seek(t);
            self.roll.follow_to(t);
        }
    }

    fn toggle_play(&mut self) {
        if let Some(pb) = &self.pb {
            if !self.playing && pb.ran >= pb.frames && self.recording {
                // Starting over from frame 0 would record over the segment from its start.
                if !self.rec_extending() {
                    self.info("Recording is at the end of the segment: turn on Extend segment (Record menu) to record past it, or move the playhead back");
                    return;
                }
            } else if !self.playing && pb.ran >= pb.frames {
                self.seek(0);
            }
            self.playing = !self.playing;
            self.clock = 0.0;
        }
    }

    /// Double-click in the tree: select the segment if needed and play it.
    /// While the segment is still loading, playback starts once it is ready.
    fn play_node(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.select(id);
        }
        if self.pb.is_none() {
            self.playing = self.job.is_some();
            self.clock = 0.0;
        } else if !self.playing {
            self.toggle_play();
        }
    }

    fn at_end(&self) -> bool {
        self.pb.as_ref().is_some_and(|p| p.ran >= p.frames)
    }

    fn tick(&mut self, dt: f64) {
        if !self.playing || self.job.is_some() || self.hydrating.is_some() {
            self.clock = 0.0;
            return;
        }
        // Playback is running: the next edit starts a new "since playback last ran".
        self.edit_from = None;
        // For "Stop at a changed ending": whether the segment had its ! before these frames.
        let changed_before = self.selected.as_deref().is_some_and(|id| self.ending_changed(id));
        let speed = SPEEDS[self.speed].0;
        // Real-time recording: what is held now goes into every frame this tick plays.
        let chord = if self.recording && !self.rec_cfg.frame_by_frame { self.held_chord() } else { None };
        let res = if speed == 0.0 {
            // As fast as the core goes, drawing once per screen refresh.
            let t0 = std::time::Instant::now();
            let mut r = Ok(());
            while (!self.at_end() || self.rec_extending()) && r.is_ok() {
                let last = t0.elapsed() >= std::time::Duration::from_millis(14);
                if !self.record_live(chord.as_deref()) {
                    break;
                }
                r = self.run_one(last);
                if last || !self.playing {
                    break;
                }
            }
            r
        } else {
            let rate = self.fps * speed;
            self.clock += dt;
            let mut n = (self.clock * rate).floor() as u64;
            self.clock -= n as f64 / rate;
            // Never fall far behind; slow down instead.
            let cap = (4.0 * speed).ceil().max(1.0) as u64;
            if n > cap {
                n = cap;
                self.clock = 0.0;
            }
            let mut r = Ok(());
            for i in 0..n {
                if !self.record_live(chord.as_deref()) {
                    break;
                }
                r = self.run_one(i + 1 == n);
                if r.is_err() || !self.playing {
                    break;
                }
            }
            r
        };
        if let Err(e) = res {
            self.playing = false;
            self.error(e);
            return;
        }
        if !self.playing {
            // Stopped by the comparison core (a parity failure, a failed tick or sync).
            return;
        }
        if self.at_end() && self.recording {
            // Recording stays on at the end of the segment; playback pauses there.
            if !self.rec_extending() {
                self.playing = false;
            }
            return;
        }
        if self.at_end() && self.stop_on_changed && !changed_before {
            // Only the first changed ending of a run: below it, a change is expected.
            let parent_changed = |id: &str| {
                let parent = self.col.as_ref().and_then(|c| c.replays.get(id)).and_then(|r| r.parent().map(String::from));
                parent.is_some_and(|p| self.ending_changed(&p))
            };
            if let Some(id) = self.selected.clone().filter(|id| self.ending_changed(id) && !parent_changed(id)) {
                self.playing = false;
                self.route.clear();
                self.route_target = None;
                let title = col_title(self.col.as_ref(), &id);
                self.info(format!("Stopped: \"{title}\" ended differently from its saved file (!). Undo now, before the segments below replay, or play on."));
                return;
            }
        }
        if self.at_end() {
            if !self.route.is_empty() {
                let next = self.route.remove(0);
                if let Err(e) = self.continue_into(&next) {
                    self.playing = false;
                    self.error(e);
                }
                return;
            }
            if self.route_target.is_some() && self.route_target == self.selected {
                self.route_target = None;
                self.playing = false;
                return;
            }
            match self.end_mode {
                EndMode::Stop => self.playing = false,
                EndMode::Loop => {
                    self.seek(0);
                    self.roll.follow_to(0);
                }
                EndMode::Next => match self.next_node() {
                    Some(child) => {
                        if let Err(e) = self.continue_into(&child) {
                            self.playing = false;
                            self.error(e);
                        }
                    }
                    None => self.playing = false,
                },
                EndMode::List => self.list_next(),
            }
        }
    }

    /// Plays through to the end of `target`: on from the playhead when the current segment is
    /// on the way there, otherwise from the start of the first segment below where the two
    /// paths meet (or from the trunk).
    fn play_to(&mut self, target: &str) {
        let Some(col) = &self.col else { return };
        let Ok(path) = col.chain(target) else { return };
        let path: Vec<String> = path.into_iter().map(String::from).collect();
        let cur = self.selected.clone();
        let on_path = cur.as_ref().and_then(|c| path.iter().position(|p| p == c));
        let (start, from_playhead) = match on_path {
            Some(i) => (i, true),
            None => {
                let cur_chain: Vec<String> = cur.as_deref().and_then(|c| col.chain(c).ok()).unwrap_or_default().into_iter().map(String::from).collect();
                let common = path.iter().zip(cur_chain.iter()).take_while(|(a, b)| a == b).count();
                // `common` segments are shared; start just below them, or at the target if it is one of them.
                (common.min(path.len() - 1), false)
            }
        };
        if !from_playhead {
            self.select(&path[start]);
        } else if self.at_end() && start == path.len() - 1 {
            self.seek(0);
        }
        self.route = path[start + 1..].to_vec();
        self.route_target = Some(target.to_string());
        self.playing = self.pb.is_some();
        self.clock = 0.0;
        let title = col_title(self.col.as_ref(), target);
        self.info(format!("Playing to the end of \"{title}\" ({} segment(s) on the way)", self.route.len() + 1));
    }

    /// Where continue goes from the current segment: the sticky child, or else the only
    /// child, saved or not; None if it is agent-owned. The core is already where the child
    /// starts, whatever the files say.
    fn next_node(&self) -> Option<String> {
        let (col, pb) = (self.col.as_ref()?, self.pb.as_ref()?);
        let next = match self.sticky_child(&pb.id) {
            Some(c) => c,
            None => {
                let kids = col.children(Some(&pb.id));
                if kids.len() != 1 {
                    return None;
                }
                kids[0].to_string()
            }
        };
        if self.agent_owned(&next) {
            return None;
        }
        Some(next)
    }

    /// Asks the agent's server to close the session that has draft `id` open, keeping the
    /// draft (a file in .local/mcp/takeover/, honoured before the agent's next tool call),
    /// and treats it as the person's from now on.
    fn take_over(&mut self, id: &str) {
        let Some(root) = self.col.as_ref().map(|c| c.root.clone()) else { return };
        let dir = root.join(".local").join("mcp").join("takeover");
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(id), "")) {
            self.error(format!("Could not take it over: {e}"));
            return;
        }
        self.taken_over.insert(id.to_string());
        self.coverage_stale();
        let title = col_title(self.col.as_ref(), id);
        self.info(format!("Took over \"{title}\": it is yours to edit and save. The agent's session closes the next time it is used."));
    }

    /// A draft an agent session has open: the agent's until the session closes.
    fn agent_owned(&self, id: &str) -> bool {
        !self.taken_over.contains(id) && self.agent_sessions.iter().any(|s| s.draft == id) && self.col.as_ref().is_some_and(|c| c.is_draft(id))
    }

    /// The segment the roll shows after the end of this one: the sticky child, or else the
    /// only child.
    fn next_neighbour(&self) -> Option<String> {
        let (col, id) = (self.col.as_ref()?, self.selected.as_ref()?);
        self.sticky_child(id).or_else(|| {
            let kids = col.children(Some(id));
            (kids.len() == 1).then(|| kids[0].to_string())
        })
    }

    /// Replay everything: ordinary playback in List mode from frame 0 of the first segment in
    /// tree order (the focused segment, if the tree is focused), so it can be paused and watched.
    fn replay_everything(&mut self) {
        let Some(col) = &self.col else { return };
        let first = match self.hoist.as_deref().filter(|h| col.replays.contains_key(*h)) {
            Some(h) => Some(h.to_string()),
            None => col.tree_order().first().map(|(id, _)| id.to_string()),
        };
        let Some(first) = first else {
            self.info("No segments to replay");
            return;
        };
        self.end_mode = EndMode::List;
        self.route.clear();
        self.route_target = None;
        if self.selected.as_deref() != Some(first.as_str()) {
            self.select(&first);
        }
        self.seek(0);
        self.roll.follow_to(0);
        self.playing = true;
        self.clock = 0.0;
        let what = if self.hoist.is_some() { "the focused branch" } else { "every segment" };
        self.info(format!("Replaying {what} in List mode at {}: pause with Space, or pick MAX speed to go fastest. Changed endings are marked !.", SPEEDS[self.speed].1));
    }

    /// List mode, at the end of a segment: go on to the next segment in tree order (within
    /// the focused branch, if the tree is focused), straight on into it when it is a child of
    /// this one, otherwise from its parent's end state.
    fn list_next(&mut self) {
        let (Some(col), Some(pb)) = (&self.col, &self.pb) else { return };
        let order: Vec<&str> = col.tree_order().into_iter().map(|(id, _)| id).collect();
        let scope: Vec<&str> = match self.hoist.as_deref().filter(|h| col.replays.contains_key(*h)) {
            Some(h) => order.iter().copied().filter(|id| col.chain(id).is_ok_and(|c| c.contains(&h))).collect(),
            None => order,
        };
        let next = scope.iter().position(|id| *id == pb.id).and_then(|i| scope.get(i + 1)).map(|s| s.to_string());
        let Some(next) = next else {
            self.playing = false;
            self.info(if self.hoist.is_some() { "List: reached the end of the focused branch" } else { "List: reached the end of the segment list" });
            return;
        };
        let is_child = col.replays[&next].parent() == Some(pb.id.as_str());
        if is_child {
            if let Err(e) = self.continue_into(&next) {
                self.playing = false;
                self.error(e);
            }
        } else {
            // A sibling of this segment or of an ancestor: select it (from its parent's
            // stored end state, or by playing the chain) and keep playing once it is ready.
            self.select(&next);
            self.playing = self.pb.is_some() || self.job.is_some();
            self.clock = 0.0;
        }
    }

    /// The sticky tip, if it still exists.
    fn tip(&self) -> Option<&str> {
        let t = self.sticky_tip.as_deref()?;
        self.col.as_ref()?.replays.contains_key(t).then_some(t)
    }

    /// The tip's chain from the root segment, when sticky is on.
    fn sticky_path(&self) -> Vec<String> {
        match (self.col.as_ref(), self.tip()) {
            (Some(col), Some(t)) => col.chain(t).map(|c| c.into_iter().map(String::from).collect()).unwrap_or_default(),
            _ => vec![],
        }
    }

    /// The child of `id` on the way to the sticky tip (sticky on, `id` above the tip).
    fn sticky_child(&self, id: &str) -> Option<String> {
        let path = self.sticky_path();
        let i = path.iter().position(|p| p == id)?;
        path.get(i + 1).cloned()
    }

    /// Keeps the tip while the selection stays on its chain; anything else starts a new one.
    fn update_sticky_tip(&mut self) {
        let Some(sel) = self.selected.clone() else { return };
        let Some(col) = &self.col else { return };
        if !col.replays.contains_key(&sel) {
            return;
        }
        // A deleted tip falls back to its nearest surviving ancestor: the selection is shown
        // there, so starting from the selection is the same thing.
        let keep = self.tip().is_some_and(|t| col.chain(t).is_ok_and(|c| c.contains(&sel.as_str())));
        if !keep {
            self.sticky_tip = Some(sel);
        }
    }

    /// Moves from the end of the current segment straight into `child`, keeping the core's state.
    fn continue_into(&mut self, child: &str) -> Result<()> {
        let (Some(col), Some(s), Some(pb)) = (&self.col, &mut self.session, &self.pb) else { return Ok(()) };
        let start_frame = pb.start_frame + pb.frames;
        // Straight on: the baseline (and the comparison core, if in step) carry on into the child.
        let end = Some((pb.id.clone(), pb.frames));
        if self.base_next == end {
            self.base_next = Some((child.to_string(), 0));
        }
        if self.checker.at == end {
            self.checker.lose_step();
            if let Some(c) = &mut self.compare {
                match c.enter(col, child) {
                    Ok(()) => self.checker.at = Some((child.to_string(), 0)),
                    Err(e) => self.compare_problem = Some(format!("{e:#}")),
                }
            }
        }
        let rec = start_pass(s, col, child, start_frame)?;
        let start_hash = Some(rec.start_hash.clone());
        let snap = s.snapshot()?;
        let r = &col.replays[child];
        let frames = r.frames().unwrap_or(0);
        let checkpoints = r.checkpoints().unwrap_or_default().into_iter().collect();
        self.pb = Some(Playback { id: child.to_string(), start_frame, frames, ran: 0, snapshots: BTreeMap::from([(0, snap)]), checkpoints, checkpoint_miss: None, checkpoints_seen: BTreeMap::new(), end_hash: None, start_hash, stored: vec![], stale_from: None, picture_stale: false, rec: Some(rec) });
        self.edit_from = None;
        self.selected = Some(child.to_string());
        self.roll.follow_to(0);
        self.coverage_restart_live();
        Ok(())
    }

    // ---- comparison cores (docs/gui.md#comparison-cores) ----

    fn unload_compare(&mut self) {
        self.parity_save(true);
        self.parity = None;
        self.rerun = None;
        self.compare = None;
        self.checker.lose_step();
        self.compare_failed = false;
        self.checker.synced = None;
        self.compare_problem = None;
        self.compare_texture = None;
        self.compare_need_sync = false;
    }

    /// Compare: loads comparison core `name` from `[[compare]]` (unloading the one before
    /// it first), or none.
    fn select_compare(&mut self, name: Option<String>) {
        self.unload_compare();
        let (Some(name), Some(col)) = (name, &self.col) else { return };
        let local = col.settings().unwrap_or_default();
        let rom = local.rom.clone();
        let core = local.core.clone().unwrap_or_default();
        match RemoteComparison::open(col, &name, &core, rom.as_deref()) {
            Ok(c) => {
                self.compare = Some(c);
                self.compare_need_sync = true;
                self.parity_load();
                self.info(format!("Loaded comparison core \"{name}\""));
            }
            Err(e) => self.compare_dialog = Some(format!("Could not load comparison core \"{name}\": {e:#}")),
        }
    }

    /// Brings the comparison core to the baseline's state at frame `k` of `id` (before the
    /// baseline runs it).
    fn compare_sync(&mut self, id: &str, k: u64) {
        self.compare_need_sync = false;
        let (Some(col), Some(s), Some(pb), Some(c)) = (&self.col, &mut self.session, &self.pb, &mut self.compare) else { return };
        match self.checker.sync(c, &mut s.core, col, id, k, pb.start_frame + k) {
            Ok(()) => self.compare_problem = None,
            Err(msg) => {
                self.compare_problem = Some(msg);
                // Any failure stops playback; playing on carries on without the comparison
                // (the sync is tried again at the next jump, or with Sync now).
                self.playing = false;
            }
        }
    }

    /// Paused with a sync wanted (after a jump, Run comparison or Sync now): re-run the frame
    /// shown, syncing just before it, so both pictures show the same frame.
    fn compare_idle(&mut self, ctx: &egui::Context) {
        if !self.compare_need_sync || !self.compare_run || self.compare.is_none() || self.playing || self.job.is_some() || self.hydrating.is_some() {
            return;
        }
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let Some(shown) = self.pb.as_ref().and_then(|pb| pb.shown()) else { return };
        self.reshow = true;
        self.seek(shown);
        self.reshow = false;
        // Tried once (the re-run syncs before its last frame), whatever came of it.
        self.compare_need_sync = false;
    }

    // ---- editing ----

    // ---- recording (docs/gui.md#recording) ----

    /// Why Record can't be turned on now, if it can't.
    fn record_blocked(&self) -> Option<&'static str> {
        let Some(id) = &self.selected else { return Some("Select a segment to record into") };
        if self.agent_owned(id) {
            return Some("This draft is open in an agent session and is read-only: right-click it and Take over to record into it");
        }
        if self.pb.is_none() || self.hydrating.is_some() {
            return Some("Busy: wait for the segment to load or the clean to finish");
        }
        None
    }

    fn set_recording(&mut self, on: bool) {
        let on = on && self.record_blocked().is_none();
        if on != self.recording && on {
            let how = if self.rec_cfg.frame_by_frame { "commit a frame with the commit key, step back with rewind" } else { "play to record what you hold" };
            self.info(format!("Recording into player {}: {how}. Escape stops.", self.rec_cfg.port + 1));
        }
        self.recording = on;
        self.roll.recording = on;
        self.rec_pass = false;
        self.rec_live = false;
        self.rec_hold = None;
        self.rec_resume = false;
    }

    /// Real-time recording past the last frame makes the segment longer.
    fn rec_extending(&self) -> bool {
        self.recording && self.rec_cfg.extend && !self.rec_cfg.frame_by_frame
    }

    fn save_record_settings(&mut self) {
        if let Some(col) = &self.col {
            let mut local = col.load_local().unwrap_or_default();
            local.record = Some(self.rec_cfg.to_local());
            if let Err(e) = col.save_local(&local) {
                self.error(e);
            }
        }
    }

    /// The buttons held on the keyboard now, on the controller being recorded. None while
    /// the keyboard belongs to a text field or a dialog: nothing is recorded then.
    fn held_chord(&self) -> Option<Vec<InputKey>> {
        let ctx = egui_ctx();
        if ctx.egui_wants_keyboard_input() || self.mapping.is_some() {
            return None;
        }
        let held = ctx.input(|i| i.keys_down.clone());
        Some(self.bindings.chord(&held, self.rec_cfg.port))
    }

    /// Joypad controllers to record into: the project's, and always players 1 and 2.
    fn record_ports(&self) -> Vec<u32> {
        let mut ports: BTreeSet<u32> = [0, 1].into_iter().collect();
        if let Some(col) = &self.col {
            for p in &col.config.ports {
                if p.port > 0 && decomp_dj::format::device_from_name(&p.device).is_ok_and(|d| d & 0xff == decomp_dj::format::DEVICE_JOYPAD) {
                    ports.insert(p.port - 1);
                }
            }
        }
        ports.insert(self.rec_cfg.port);
        ports.into_iter().collect()
    }

    /// Writes frame `f` of the current segment before it runs: the buttons in `chord`, or
    /// nothing held when `chord` is None. At the end, with Extend segment on, the segment
    /// grows by this frame. Returns false when the frame can't be written.
    fn record_frame(&mut self, f: u64, chord: Option<&[InputKey]>) -> bool {
        let Some(pb) = &self.pb else { return false };
        let (id, frames) = (pb.id.clone(), pb.frames);
        if self.agent_owned(&id) {
            self.set_recording(false);
            return false;
        }
        let Some(col) = &self.col else { return false };
        let mut r = col.replays[&id].clone();
        let grow = f >= frames;
        if grow {
            if !self.rec_cfg.extend || f > frames {
                return false;
            }
            if let Err(e) = r.set_frames(f + 1) {
                self.error(e);
                return false;
            }
        }
        let changed = match chord {
            Some(c) => record::write_frame(&mut r, f, c, self.rec_cfg.overwrite),
            None => record::clear_frame(&mut r, f),
        };
        match changed {
            Ok(false) if !grow => return true,
            Ok(_) => {}
            Err(e) => {
                self.error(e);
                return false;
            }
        }
        // One undo step per pass, taken before its first change.
        if !self.rec_pass {
            if !self.remember("Record inputs", &[&id]) {
                self.set_recording(false);
                return false;
            }
            self.rec_pass = true;
        }
        self.col.as_mut().unwrap().replays.insert(id.clone(), r);
        if grow {
            if let Some(pb) = &mut self.pb {
                pb.frames = f + 1;
            }
        }
        self.record_invalidate(&id, f);
        true
    }

    /// `invalidate_from` for a frame about to run: the same bookkeeping, but nothing is run
    /// again (frames before `from` are unchanged) and playback isn't paused.
    fn record_invalidate(&mut self, id: &str, from: u64) {
        let Some(col) = &mut self.col else { return };
        let r = col.replays.get_mut(id).unwrap();
        r.remove_all("end-hash");
        let keep: Vec<(u64, String)> = r.checkpoints().unwrap_or_default().into_iter().filter(|(f, _)| *f <= from).collect();
        r.remove_all("checkpoint");
        for (f, h) in &keep {
            r.add("checkpoint", &format!("{f} {h}"));
        }
        self.edited.insert(id.to_string());
        let e = self.dirty_from.entry(id.to_string()).or_insert(from);
        *e = (*e).min(from);
        self.roll.invalidate();
        let Some(pb) = &mut self.pb else { return };
        if let (Some(rec), Some(s)) = (pb.rec.take(), &mut self.session) {
            let _ = s.keep_snapshots(rec);
        }
        pb.snapshots.retain(|k, _| *k <= from);
        pb.checkpoints = keep.into_iter().collect();
        pb.checkpoint_miss = pb.checkpoint_miss.filter(|m| *m <= from);
        pb.end_hash = None;
        pb.stale_from = Some(pb.stale_from.map_or(from, |s| s.min(from))).filter(|s| *s < pb.frames);
        if !self.rec_live {
            self.rec_live = true;
            self.coverage_restart_live();
        }
    }

    /// Real time: writes the held buttons into the frame about to play. False: stop here.
    fn record_live(&mut self, chord: Option<&[InputKey]>) -> bool {
        let Some(chord) = chord else { return true };
        if !self.recording || self.rec_cfg.frame_by_frame {
            return true;
        }
        let Some(f) = self.pb.as_ref().map(|p| p.ran) else { return true };
        self.record_frame(f, Some(chord))
    }

    /// Frame by frame: commit writes the held buttons to the next frame and runs it;
    /// rewind steps back one frame (clearing the frame it leaves when asked). Holding
    /// either repeats at the playback speed after a short delay.
    fn record_keys(&mut self, ctx: &egui::Context, dt: f64) {
        if !self.recording {
            return;
        }
        if self.record_blocked().is_some() {
            self.set_recording(false);
            return;
        }
        if ctx.egui_wants_keyboard_input() || self.mapping.is_some() {
            self.rec_hold = None;
            self.rec_resume = false;
            return;
        }
        // A focused button would take Space or Enter as a click.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        // In real time the commit key plays and pauses (elsewhere); rewind steps back here
        // in both modes.
        let fbf = self.rec_cfg.frame_by_frame;
        let (commit, rewind, shift) = ctx.input(|i| (fbf && self.bindings.commit.is_some_and(|k| i.key_down(k)), self.bindings.rewind.is_some_and(|k| i.key_down(k)), i.modifiers.shift));
        let which = if commit { Some(true) } else if rewind { Some(false) } else { None };
        let Some(c) = which else {
            // A held commit or rewind is one pass, and one undo step.
            if self.rec_hold.take().is_some() {
                self.rec_pass = false;
                self.rec_live = false;
                // Real time: recording carries on forward from where rewind was let go.
                if !fbf && std::mem::take(&mut self.rec_resume) {
                    self.playing = true;
                }
            }
            return;
        };
        if !fbf && self.rec_hold.is_none() {
            self.rec_resume = self.playing;
        }
        let steps = match &mut self.rec_hold {
            Some((was, held, clock)) if *was == c => {
                *held += dt;
                if *held < REC_HOLD_DELAY {
                    0
                } else {
                    let speed = SPEEDS[self.speed].0;
                    if speed == 0.0 {
                        1
                    } else {
                        let rate = self.fps * speed;
                        *clock += dt;
                        let n = (*clock * rate).floor();
                        *clock -= n / rate;
                        (n as u64).min(4)
                    }
                }
            }
            _ => {
                if self.rec_hold.is_some() {
                    self.rec_pass = false;
                }
                self.rec_hold = Some((c, 0.0, 0.0));
                1
            }
        };
        ctx.request_repaint();
        let chord = if c { self.held_chord().unwrap_or_default() } else { vec![] };
        for _ in 0..steps {
            let ok = if c { self.commit_frame(&chord) } else { self.rewind_frame(shift) };
            if !ok {
                break;
            }
        }
    }

    fn commit_frame(&mut self, chord: &[InputKey]) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.playing = false;
        let Some(f) = self.pb.as_ref().map(|p| p.ran) else { return false };
        if !self.record_frame(f, Some(chord)) {
            if self.recording && self.pb.as_ref().is_some_and(|p| f >= p.frames) {
                self.info("At the end of the segment: turn on Extend segment (Record menu) to record past it");
            }
            return false;
        }
        if let Err(e) = self.run_one(true) {
            self.error(e);
            return false;
        }
        self.roll.follow_to(f);
        true
    }

    fn rewind_frame(&mut self, shift: bool) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.playing = false;
        let Some(s) = self.pb.as_ref().and_then(|p| p.shown()) else { return false };
        if s == 0 {
            return false;
        }
        if self.rec_cfg.rewind_clears != shift {
            self.record_frame(s, None);
        }
        self.seek(s - 1);
        self.roll.follow_to(s - 1);
        true
    }

    /// The record button (a red circle, as wide as play) and its options, in the transport.
    fn record_button(&mut self, ui: &mut egui::Ui, icon: egui::Vec2) {
        let blocked = self.record_blocked();
        let mut changed = false;
        let mut open_mapping = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let cr = ui.visuals().widgets.inactive.corner_radius;
            let left = egui::CornerRadius { nw: cr.nw, sw: cr.sw, ne: 0, se: 0 };
            let right = egui::CornerRadius { nw: 0, sw: 0, ne: cr.ne, se: cr.se };
            let mut b = egui::Button::new("").min_size(egui::vec2(34.0, icon.y)).corner_radius(left);
            if self.recording {
                b = b.fill(REC_RED);
            }
            let tip = if self.recording {
                "Recording: click or press Escape to stop".to_string()
            } else {
                format!("Record inputs from the keyboard into player {} ({})", self.rec_cfg.port + 1, if self.rec_cfg.frame_by_frame { "frame by frame" } else { "real time" })
            };
            let r = ui.add_enabled(self.recording || blocked.is_none(), b).on_hover_text(tip).on_disabled_hover_text(blocked.unwrap_or(""));
            let dot = if self.recording {
                Color32::WHITE
            } else if r.enabled() {
                REC_RED
            } else {
                REC_RED.gamma_multiply(0.4)
            };
            ui.painter().circle_filled(r.rect.center(), 5.5, dot);
            if r.clicked() {
                self.set_recording(!self.recording);
            }
            ui.add_space(1.0);
            let arrow = egui::Button::new("").min_size(egui::vec2(16.0, icon.y)).corner_radius(right);
            let ports = self.record_ports();
            let r = egui::containers::menu::MenuButton::from_button(arrow)
                .ui(ui, |ui| {
                    let cfg = &mut self.rec_cfg;
                    changed |= ui.checkbox(&mut cfg.frame_by_frame, "Frame by frame").keep_open().on_hover_text("Off: real time, Space plays and pauses and every played frame records what you hold. On: the commit key (Space) writes what you hold to the next frame and steps to it; rewind (Backspace) steps back").changed();
                    changed |= ui.checkbox(&mut cfg.rewind_clears, "Rewind clears inputs").keep_open().on_hover_text("Rewind also clears the input on each frame it leaves. Shift+rewind does the opposite").changed();
                    changed |= ui.checkbox(&mut cfg.overwrite, "Overwrite all inputs").keep_open().on_hover_text("Each recorded frame clears every controller's input first, so only what you hold is left. Off: held buttons are only added (to record player 2 over player 1, say)").changed();
                    changed |= ui.checkbox(&mut cfg.extend, "Extend segment").keep_open().on_hover_text("Recording past the last frame makes the segment longer. Off: playback pauses at the end and recording stays on").changed();
                    ui.separator();
                    ui.menu_button(format!("Controller: player {}", cfg.port + 1), |ui| {
                        for p in &ports {
                            if ui.radio(cfg.port == *p, format!("Player {}", p + 1)).clicked() {
                                cfg.port = *p;
                                changed = true;
                            }
                        }
                    });
                    if ui.button("Keyboard mapping…").clicked() {
                        open_mapping = true;
                    }
                })
                .0;
            let r = r.on_hover_text("Recording options");
            let c = icon_color(ui, &r);
            let m = r.rect.center();
            ui.painter().add(egui::Shape::convex_polygon(vec![egui::pos2(m.x - 3.5, m.y - 2.0), egui::pos2(m.x + 3.5, m.y - 2.0), egui::pos2(m.x, m.y + 2.5)], c, egui::Stroke::NONE));
        });
        if changed {
            self.rec_hold = None;
            self.rec_resume = false;
            self.rec_pass = false;
            self.save_record_settings();
        }
        if open_mapping {
            self.mapping = Some(record::MappingDialog::new(&self.bindings));
        }
    }

    fn apply_edit(&mut self, label: &str, edits: Vec<(InputKey, Vec<(u64, u64)>)>, from: u64) {
        let Some(id) = self.selected.clone() else { return };
        if !self.remember(label, &[&id]) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let r = col.replays.get_mut(&id).unwrap();
        for (key, spans) in edits {
            if let Err(e) = r.set_binary_spans(key, &spans) {
                self.error(e);
                return;
            }
        }
        self.invalidate_from(&id, from);
    }

    /// After a change at frame `from` of `id`: drops the end hash and later checkpoints
    /// (so the segment and everything below turn dirty) and redraws.
    fn invalidate_from(&mut self, id: &str, from: u64) {
        let Some(col) = &mut self.col else { return };
        let id = id.to_string();
        let r = col.replays.get_mut(&id).unwrap();
        // The end state and later checkpoints are no longer known; children turn dirty.
        r.remove_all("end-hash");
        let keep: Vec<(u64, String)> = r.checkpoints().unwrap_or_default().into_iter().filter(|(f, _)| *f <= from).collect();
        r.remove_all("checkpoint");
        for (f, h) in &keep {
            r.add("checkpoint", &format!("{f} {h}"));
        }
        self.edited.insert(id.clone());
        let e = self.dirty_from.entry(id.clone()).or_insert(from);
        *e = (*e).min(from);
        self.roll.invalidate();
        let here = self.playhead();
        if let Some(pb) = &mut self.pb {
            if pb.id == id {
                if let (Some(rec), Some(s)) = (pb.rec.take(), &mut self.session) {
                    let _ = s.keep_snapshots(rec);
                }
                pb.snapshots.retain(|k, _| *k <= from);
                pb.checkpoints = keep.into_iter().collect();
                pb.checkpoint_miss = pb.checkpoint_miss.filter(|m| *m <= from);
                pb.end_hash = None;
                let cur = here;
                // Live coverage starts again: the frames after the edit run differently now.
                self.coverage_restart_live();
                self.show_after_edit(from, cur);
            }
        }
    }

    /// After the current segment changed from frame `from` on (its snapshots past `from`
    /// already dropped), with `cur` the frame shown before the change: marks what is out of
    /// date, then either returns to the edit (pauses, shows the earliest frame edited since
    /// playback last ran) or redraws `cur`.
    fn show_after_edit(&mut self, from: u64, cur: Option<u64>) {
        // The person moved the playhead since the last edit: this edit starts a new run, so
        // it returns to itself, not to an earlier edit somewhere else.
        if cur != self.edit_at {
            self.edit_from = None;
        }
        let Some(pb) = &mut self.pb else { return };
        pb.stale_from = Some(pb.stale_from.map_or(from, |s| s.min(from))).filter(|s| *s < pb.frames);
        if cur.is_some_and(|c| c >= from) {
            pb.picture_stale = true;
        }
        let last = pb.frames.saturating_sub(1);
        let target = if self.return_to_edit {
            self.playing = false;
            let e = self.edit_from.map_or(from, |e| e.min(from));
            self.edit_from = Some(e);
            self.edit_at = Some(e.min(last));
            self.edit_at
        } else {
            cur
        };
        if let Some(t) = target {
            // Run again from a snapshot at or before the edit.
            if let Some(pb) = &mut self.pb {
                pb.ran = 0;
            }
            self.seek(t);
            if self.return_to_edit {
                self.roll.follow_to(t);
            }
        }
    }

    // ---- undo, redo and clipboard ----

    /// Records the segments `ids` as they are now, before a change, as one undo step.
    /// Returns false, changing nothing, when one of them is agent-owned: those are read-only
    /// until taken over.
    fn remember(&mut self, label: &str, ids: &[&str]) -> bool {
        if let Some(id) = ids.iter().find(|id| self.agent_owned(id)) {
            let title = col_title(self.col.as_ref(), id);
            self.info(format!("\"{title}\" is open in an agent session: right-click it and Take over to edit it."));
            return false;
        }
        let Some(col) = &self.col else { return false };
        let segments = ids.iter().map(|id| (id.to_string(), col.replays.get(*id).cloned())).collect();
        let frame = self.pb.as_ref().and_then(|p| p.shown());
        self.undo.push(UndoStep { label: label.to_string(), segments, selected: self.selected.clone(), frame });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.coverage_stale();
        true
    }

    /// The frame the playhead is on, or is on its way to when a seek is running.
    fn playhead(&self) -> Option<u64> {
        match &self.job {
            Some(Job::Seek { to, .. }) => to.checked_sub(1),
            _ => self.pb.as_ref().and_then(|p| p.shown()),
        }
    }

    /// Hashes a clean recorded become one undo step, so undo takes them back.
    fn undo_clean(&mut self, hy: &mut Hydrating) {
        let mut segments: Vec<(String, Option<decomp_dj::format::Replay>)> = vec![];
        for e in hy.h.events.iter().filter(|e| matches!(e.verdict, "recorded" | "cleaned" | "changed")) {
            if let Some(r) = hy.before.remove(&e.id) {
                segments.push((e.id.clone(), Some(r)));
            }
        }
        if segments.is_empty() {
            return;
        }
        let frame = self.pb.as_ref().and_then(|p| p.shown());
        self.undo.push(UndoStep { label: format!("Clean {}", hy.what), segments, selected: self.selected.clone(), frame });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Hashes recorded because of the last change (reaching a segment's end after an edit,
    /// say) join the last undo step, so one undo takes back the change and what it caused.
    /// `before` is the segment as it was before its hashes were recorded.
    fn fold_into_undo(&mut self, id: &str, before: decomp_dj::format::Replay) {
        if let Some(step) = self.undo.last_mut() {
            if !step.segments.iter().any(|(s, _)| s == id) {
                step.segments.push((id.to_string(), Some(before)));
            }
        }
    }

    fn undo(&mut self) {
        self.rec_pass = false;
        if let Some(step) = self.undo.pop() {
            let label = step.label.clone();
            let back = self.restore(step);
            self.redo.push(back);
            self.info(format!("Undid: {label}"));
        }
    }

    fn redo(&mut self) {
        self.rec_pass = false;
        if let Some(step) = self.redo.pop() {
            let label = step.label.clone();
            let back = self.restore(step);
            self.undo.push(back);
            self.info(format!("Redid: {label}"));
        }
    }

    /// Puts the segments of `step` back and returns the step that reverses it.
    fn restore(&mut self, step: UndoStep) -> UndoStep {
        self.coverage_stale();
        self.renaming = None;
        self.playing = false;
        let here = self.playhead();
        let cur = self.selected.clone();
        let Some(col) = &mut self.col else { return step };
        let chain: Vec<String> = cur.as_deref().and_then(|c| col.chain(c).ok()).unwrap_or_default().into_iter().map(String::from).collect();
        // A change to the current segment alone, keeping its length and parent, only needs the
        // frames from the first difference on to be run again. Anything else reloads the segment.
        let mut first_diff: Option<u64> = None;
        let mut reload = false;
        let mut back = vec![];
        for (id, before) in step.segments {
            let now = col.replays.get(&id).cloned();
            if cur.as_deref() == Some(id.as_str()) {
                match (&now, &before) {
                    // A root's save file or snapshot decides its first frame.
                    (Some(a), Some(b))
                        if a.frames().ok() == b.frames().ok()
                            && a.parent() == b.parent()
                            && ["sram", "snapshot"].iter().all(|f| a.get(f) == b.get(f)) =>
                    {
                        if let Some(f) = first_change(a, b) {
                            first_diff = Some(first_diff.map_or(f, |d| d.min(f)));
                        }
                    }
                    _ => reload = true,
                }
            } else if chain.contains(&id) {
                reload = true;
            }
            match before {
                Some(r) => {
                    col.replays.insert(id.clone(), r);
                }
                None => {
                    col.replays.remove(&id);
                }
            }
            self.edited.insert(id.clone());
            back.push((id, now));
        }
        let reverse = UndoStep { label: step.label, segments: back, selected: cur.clone(), frame: here };
        self.roll.invalidate();
        let target = step.selected.filter(|s| col.replays.contains_key(s));
        if self.hoist.as_ref().is_some_and(|h| !col.replays.contains_key(h)) {
            self.hoist = None;
        }
        if target != cur || reload {
            match &target {
                Some(t) => {
                    self.select(t);
                    let f = step.frame.unwrap_or(0);
                    self.seek(f);
                    self.roll.follow_to(f);
                }
                None => {
                    self.selected = None;
                    { self.end_pass(); self.pb = None; }
                    self.texture = None;
                }
            }
        } else if let (Some(from), Some(id)) = (first_diff, &cur) {
            // Same segment: drop what playback knew past the first change and redraw here.
            let checkpoints: BTreeMap<u64, String> = col.replays[id].checkpoints().unwrap_or_default().into_iter().collect();
            self.end_pass();
            if let Some(pb) = &mut self.pb {
                pb.snapshots.retain(|k, _| *k <= from);
                pb.checkpoints = checkpoints;
                pb.checkpoint_miss = None;
                pb.end_hash = None;
            }
            self.show_after_edit(from, here);
        }
        let ids: Vec<String> = reverse.segments.iter().map(|(id, _)| id.clone()).collect();
        self.settle_edited(&ids);
        // What the live play ran described the content before the step.
        if self.selected.as_ref().is_some_and(|s| ids.contains(s)) {
            self.coverage_restart_live();
        }
        self.refresh_frame_dialog();
        reverse
    }

    /// Unsaved means different from the file: segments in `ids` that are back to exactly
    /// what their saved file holds (a redo after saving, say) stop being unsaved.
    fn settle_edited(&mut self, ids: &[String]) {
        let Some(col) = &self.col else { return };
        for id in ids {
            let (Some(r), Some(path)) = (col.replays.get(id), col.paths.get(id)) else { continue };
            if col.is_draft(id) {
                continue;
            }
            if std::fs::read_to_string(path).is_ok_and(|disk| disk == r.to_text()) {
                self.edited.remove(id);
            }
        }
    }

    fn copy(&mut self) -> bool {
        let notes = self.roll.selected_notes();
        let Some(base) = notes.iter().map(|(_, sp)| sp.0).min() else { return false };
        let n = notes.len();
        // Also put the notes on the system clipboard as replay lines (frames relative to the
        // first note). Some platforms only report Ctrl/Cmd+V when the clipboard holds text.
        let mut lines: Vec<(u64, String)> = vec![];
        for (k, sp) in &notes {
            lines.push((sp.0 - base, decomp_dj::format::input_text(*k, 1)));
            lines.push((sp.1 - base, decomp_dj::format::input_text(*k, 0)));
        }
        lines.sort();
        let text: Vec<String> = lines.into_iter().map(|(f, t)| format!("{f} {t}")).collect();
        self.ctx_copy = Some(text.join("\n") + "\n");
        self.clip = Some(Clip { notes: notes.into_iter().map(|(k, sp)| (k, sp.0 - base, sp.1 - sp.0)).collect() });
        self.info(format!("Copied {n} note{}", if n == 1 { "" } else { "s" }));
        true
    }

    fn cut(&mut self) {
        let notes = self.roll.selected_notes();
        if notes.is_empty() || !self.copy() {
            return;
        }
        let (Some(col), Some(id)) = (&self.col, &self.selected) else { return };
        let r = &col.replays[id];
        let keys: BTreeSet<InputKey> = notes.iter().map(|(k, _)| *k).collect();
        let edits = keys.into_iter().map(|k| (k, r.binary_spans(k).into_iter().filter(|sp| !notes.contains(&(k, *sp))).collect())).collect();
        let from = notes.iter().map(|(_, sp)| sp.0).min().unwrap_or(0);
        self.roll.set_selection([]);
        self.apply_edit("Cut", edits, from);
    }

    /// Pastes the copied notes so the leftmost one starts at the play cursor. The pasted
    /// notes become the selection; notes past the segment's end are cut off.
    fn paste(&mut self) {
        let (Some(clip), Some(col), Some(id), Some(pb)) = (&self.clip, &self.col, &self.selected, &self.pb) else { return };
        let at = pb.shown().unwrap_or(0);
        let r = &col.replays[id];
        let frames = r.frames().unwrap_or(0);
        let keys: BTreeSet<InputKey> = clip.notes.iter().map(|(k, _, _)| *k).collect();
        let mut edits = vec![];
        let mut pasted = vec![];
        for key in keys {
            let mut spans = r.binary_spans(key);
            let mut starts = vec![];
            for (_, rel, len) in clip.notes.iter().filter(|(k, _, _)| *k == key) {
                let a = at + rel;
                if a < frames {
                    spans.push((a, (a + len).min(frames)));
                    starts.push(a);
                }
            }
            if starts.is_empty() {
                continue;
            }
            let spans = roll::merge(spans);
            for a in starts {
                if let Some(sp) = spans.iter().find(|sp| sp.0 <= a && a < sp.1) {
                    pasted.push((key, sp.0));
                }
            }
            edits.push((key, spans));
        }
        if edits.is_empty() {
            self.info("Nothing to paste here: the play cursor is past the copied notes' room");
            return;
        }
        self.roll.set_selection(pasted);
        self.apply_edit("Paste", edits, at);
    }

    // ---- segments ----

    /// A segment can be joined into its parent when it is the parent's only child.
    fn can_join(&self, id: &str) -> bool {
        let Some(col) = &self.col else { return false };
        col.replays.get(id).and_then(|r| r.parent()).is_some_and(|p| col.children(Some(p)).len() == 1)
    }

    /// The only child of `id`, when it has exactly one: joining it is "Join with only child".
    fn only_child(&self, id: &str) -> Option<String> {
        let kids = self.col.as_ref()?.children(Some(id));
        (kids.len() == 1).then(|| kids[0].to_string())
    }

    /// The reverse of a split: appends `id` to its parent, which keeps its id and title and
    /// grows by `id`'s frames. The same frames run with the same inputs, so no state changes:
    /// the parent takes `id`'s end hash, and `id`'s children move up to the parent.
    /// Opens the confirmation, which can leave a marker where the parent used to end.
    fn join_with_parent(&mut self, id: &str) {
        let title = self.col.as_ref().and_then(|c| c.replays.get(id)).map(|r| r.title().to_string());
        if let Some(t) = title {
            self.join_dialog = Some((id.to_string(), true, t));
        }
    }

    fn try_join(&mut self, id: &str, marker: Option<&str>) -> Result<()> {
        self.join_step(id, marker, true)
    }

    /// The run of only-children below `top`: its only child, that one's only child, and so on,
    /// down to the first segment with no children or several.
    fn only_child_run(&self, top: &str) -> Vec<String> {
        let mut out = vec![];
        let mut cur = top.to_string();
        while let Some(c) = self.only_child(&cur) {
            out.push(c.clone());
            cur = c;
        }
        out
    }

    /// Joins the whole run of only-children below `top` into it, as one undo step. With
    /// `markers`, an event with each joined segment's title marks where it began.
    fn join_chain(&mut self, top: &str, markers: bool) -> Result<()> {
        let run = self.only_child_run(top);
        let Some(last) = run.last() else { return Err(anyhow!("this segment has no only child to join")) };
        let col = self.col.as_ref().ok_or_else(|| anyhow!("no project"))?;
        let kids: Vec<String> = col.children(Some(last)).into_iter().map(String::from).collect();
        let titles: Vec<String> = run.iter().map(|id| col.replays[id].title().to_string()).collect();
        let mut ids: Vec<&str> = vec![top];
        ids.extend(run.iter().map(String::as_str));
        ids.extend(kids.iter().map(String::as_str));
        if !self.remember("Join chain", &ids) {
            return Ok(());
        }
        for (id, title) in run.iter().zip(&titles) {
            self.join_step(id, markers.then_some(title.as_str()), false)?;
        }
        let n = run.len();
        self.info(format!("Joined {n} segment{} into \"{}\".", if n == 1 { "" } else { "s" }, col_title(self.col.as_ref(), top)));
        Ok(())
    }

    /// One join of `id` into its parent; `undo` records it as its own undo step.
    fn join_step(&mut self, id: &str, marker: Option<&str>, undo: bool) -> Result<()> {
        if !self.can_join(id) {
            return Err(anyhow!("only a segment whose parent has no other children can be joined"));
        }
        let col = self.col.as_ref().unwrap();
        let child = col.replays[id].clone();
        let pid = child.parent().unwrap().to_string();
        let kids: Vec<String> = col.children(Some(id)).into_iter().map(String::from).collect();
        let mut p = col.replays[&pid].clone();
        let (pf, cf) = (p.frames()?, child.frames()?);
        // Every input is released when a segment starts, so release what the parent still
        // holds at its end; then the child's own lines follow, shifted by the parent's length.
        let mut held: BTreeMap<InputKey, i16> = BTreeMap::new();
        for l in &p.lines {
            if let Command::Input { key, value, impulse: false } = &l.command {
                held.insert(*key, *value);
            }
        }
        p.set("frames", &(pf + cf).to_string());
        for (key, v) in held {
            if v != 0 {
                p.insert_line(pf, &decomp_dj::format::input_text(key, 0))?;
            }
        }
        if let Some(m) = marker.map(str::trim).filter(|m| !m.is_empty()) {
            p.insert_line(pf, &format!("[EVENT] {m}"))?;
        }
        for l in &child.lines {
            let t = child.line_command_text(l.line_no).ok_or_else(|| anyhow!("line {} of the child", l.line_no))?;
            p.insert_line(pf + l.frame, &t)?;
        }
        // Hashes: the child's start hash becomes a checkpoint at the join, its end hash the new end.
        p.remove_all("end-hash");
        if let Some(h) = child.get("end-hash") {
            p.set("end-hash", h);
        }
        if let Some(h) = child.get("start-hash") {
            p.add("checkpoint", &format!("{pf} {h}"));
        }
        for (f, h) in child.checkpoints()? {
            p.add("checkpoint", &format!("{} {h}", pf + f));
        }
        let mut ids: Vec<&str> = vec![pid.as_str(), id];
        ids.extend(kids.iter().map(String::as_str));
        if undo && !self.remember("Join with parent", &ids) {
            return Ok(());
        }
        let here = if self.selected.as_deref() == Some(id) { self.pb.as_ref().and_then(|b| b.shown()).map(|f| f + pf) } else { None };
        let col = self.col.as_mut().unwrap();
        col.replays.insert(pid.clone(), p);
        col.replays.remove(id);
        for k in &kids {
            col.replays.get_mut(k).unwrap().set("parent", &pid);
            self.edited.insert(k.clone());
        }
        self.edited.insert(pid.clone());
        self.edited.insert(id.to_string());
        if self.hoist.as_deref() == Some(id) {
            self.hoist = Some(pid.clone());
        }
        self.roll.invalidate();
        self.info(format!("Joined into its parent at frame {pf}."));
        if self.selected.as_deref() == Some(id) || self.selected.as_deref() == Some(pid.as_str()) {
            self.select(&pid);
            if let Some(f) = here {
                self.seek(f);
                self.roll.follow_to(f);
            }
        }
        Ok(())
    }

    /// Deletes `id` and everything below it (`ids`, from `subtree`), as one undo step.
    fn delete_nodes(&mut self, id: &str, ids: Vec<String>) {
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        if !self.remember("Delete", &refs) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let parent = col.replays.get(id).and_then(|r| r.parent().map(String::from));
        for i in &ids {
            col.replays.remove(i);
            self.edited.insert(i.clone());
            self.tree_closed.remove(i);
        }
        if self.hoist.as_ref().is_some_and(|h| ids.contains(h)) {
            self.hoist = None;
        }
        self.roll.invalidate();
        let n = ids.len();
        // If the shown segment went, show its parent instead.
        if self.selected.as_ref().is_some_and(|s| ids.contains(s)) {
            { self.end_pass(); self.pb = None; }
            self.texture = None;
            self.selected = None;
            if let Some(p) = parent {
                self.select(&p);
            }
        }
        self.info(format!("Deleted {n} segment{}. Save to remove the files, or undo.", if n == 1 { "" } else { "s" }));
    }

    /// Gives each of `ids` the parent `to` (none: a root starting from power-on). They now
    /// start from another state, so what was recorded from their start on no longer holds:
    /// their hashes go and they, and everything below, turn dirty. A save file or snapshot
    /// only belongs to a root, so a segment that gets a parent drops its own. No undo step.
    fn set_parent(&mut self, ids: &[String], to: Option<&str>) {
        let Some(col) = &mut self.col else { return };
        for id in ids {
            let Some(r) = col.replays.get_mut(id) else { continue };
            match to {
                Some(p) => {
                    r.set("parent", p);
                    r.remove_all("sram");
                    r.remove_all("snapshot");
                }
                None => r.set("parent", "none"),
            }
            r.remove_all("start-hash");
            r.remove_all("checkpoint");
            r.remove_all("end-hash");
            self.dirty_from.insert(id.clone(), 0);
            self.edited.insert(id.clone());
        }
        if let Some(p) = to {
            self.tree_closed.remove(p);
        }
        self.roll.invalidate();
        self.coverage_stale();
        // The shown segment starts from somewhere else now if it is one of them or below one.
        if let Some(sel) = self.selected.clone() {
            let moved = self.col.as_ref().and_then(|c| c.chain(&sel).ok()).is_some_and(|ch| ch.iter().any(|c| ids.iter().any(|i| i == c)));
            if moved {
                let here = self.playhead().unwrap_or(0);
                self.select(&sel);
                self.seek(here);
                self.roll.follow_to(here);
            }
        }
    }

    /// Move to…: `id` (and everything below it) goes under `to`, or becomes a root.
    fn move_node(&mut self, id: &str, to: Option<&str>) {
        let Some(col) = &self.col else { return };
        if to.is_some_and(|t| subtree_ids(col, id).contains(&t)) {
            self.info("A segment can't move below itself.");
            return;
        }
        let had_start = col.replays.get(id).is_some_and(|r| r.get("sram").is_some() || r.get("snapshot").is_some());
        let below = subtree_ids(col, id).len() - 1;
        let dest = to.map(|t| format!("under \"{}\"", col_title(Some(col), t))).unwrap_or_else(|| "to a root segment, starting from power-on".to_string());
        if !self.remember("Move", &[id]) {
            return;
        }
        self.set_parent(&[id.to_string()], to);
        let mut msg = format!("Moved {dest}. It{} must be played again to record new hashes", if below > 0 { format!(" and the {below} segment{} below it", if below == 1 { "" } else { "s" }) } else { String::new() });
        msg.push('.');
        if had_start && to.is_some() {
            msg.push_str(" Its save file or snapshot was dropped: only a root segment has one.");
        }
        self.info(msg);
    }

    /// The Move to… window: pick the new parent from every segment not below this one.
    fn move_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some((id, mut filter)) = self.move_dialog.take() else { return };
        let Some(col) = self.col.as_ref().filter(|c| c.replays.contains_key(&id)) else { return };
        let r = &col.replays[&id];
        let title = r.title().to_string();
        let parent = r.parent().map(String::from);
        let below: HashSet<&str> = subtree_ids(col, &id).into_iter().collect();
        // Every other segment as "Root › A › B", in tree order.
        let mut targets: Vec<(String, String)> = vec![];
        let mut stack: Vec<(&str, String)> = col.children(None).into_iter().rev().map(|k| (k, String::new())).collect();
        while let Some((k, pre)) = stack.pop() {
            if below.contains(k) {
                continue;
            }
            let path = if pre.is_empty() { col.replays[k].title().to_string() } else { format!("{pre} › {}", col.replays[k].title()) };
            for c in col.children(Some(k)).into_iter().rev() {
                stack.push((c, path.clone()));
            }
            targets.push((k.to_string(), path));
        }
        let needle = filter.trim().to_lowercase();
        let mut open = true;
        let mut pick: Option<Option<String>> = None;
        let mut cancel = false;
        egui::Window::new("Move segment").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label(RichText::new(format!("Move \"{title}\" and what is below it to:")).strong());
            ui.label(RichText::new("It then starts where the new parent ends, so it and everything below turn dirty.").weak());
            ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter by title").desired_width(380.0)).request_focus();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                ui.set_min_width(380.0);
                if parent.is_some() && needle.is_empty() && ui.selectable_label(false, RichText::new("No parent: a root segment, from power-on").italics()).clicked() {
                    pick = Some(None);
                }
                for (k, path) in &targets {
                    if !needle.is_empty() && !path.to_lowercase().contains(&needle) {
                        continue;
                    }
                    let here = parent.as_deref() == Some(k.as_str());
                    let label = if here { format!("{path}  (current parent)") } else { path.clone() };
                    if ui.add_enabled(!here, egui::Button::selectable(false, label)).clicked() {
                        pick = Some(Some(k.clone()));
                    }
                }
            });
            ui.add_space(4.0);
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
        match pick {
            Some(to) => self.move_node(&id, to.as_deref()),
            None if cancel || !open => {}
            None => self.move_dialog = Some((id, filter)),
        }
    }

    /// Delete, keep children: `id` goes and its children move up to its parent (or become
    /// roots), as one undo step.
    fn delete_keep_children(&mut self, id: &str) {
        let Some(col) = &self.col else { return };
        let Some(r) = col.replays.get(id) else { return };
        let parent = r.parent().map(String::from);
        let kids: Vec<String> = col.children(Some(id)).into_iter().map(String::from).collect();
        let mut ids: Vec<&str> = vec![id];
        ids.extend(kids.iter().map(String::as_str));
        if !self.remember("Delete, keep children", &ids) {
            return;
        }
        let was_shown = self.selected.as_deref() == Some(id);
        if let Some(col) = &mut self.col {
            col.replays.remove(id);
        }
        self.edited.insert(id.to_string());
        self.tree_closed.remove(id);
        if self.hoist.as_deref() == Some(id) {
            self.hoist = parent.clone();
        }
        if was_shown {
            self.end_pass();
            self.pb = None;
            self.texture = None;
            self.selected = None;
        }
        self.set_parent(&kids, parent.as_deref());
        if was_shown {
            if let Some(next) = parent.as_ref().or(kids.first()) {
                self.select(&next.clone());
            }
        }
        let n = kids.len();
        let to = if parent.is_some() { "moved up to its parent" } else { "are now root segments, starting from power-on" };
        self.info(format!("Deleted; its {n} child{} {to} and must be played again to record new hashes.", if n == 1 { "" } else { "ren" }));
    }

    fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// A new empty root segment: a cold boot with no parent (formerly a "trunk").
    fn new_root(&mut self) {
        if self.col.is_none() {
            return;
        }
        let id = Self::new_id();
        self.remember("New root segment", &[&id]);
        let Some(col) = &mut self.col else { return };
        let r = decomp_dj::format::Replay::new(&id, None, "New root segment", 600);
        col.replays.insert(id.clone(), r);
        self.edited.insert(id.clone());
        self.hoist = None;
        self.info("Added a 600-frame root segment that starts from power-on. Change its title, length and save file with Edit > Properties….");
        self.select(&id);
        self.open_node_dialog(&id);
    }

    /// Duplicate: at once for a segment with no children, otherwise ask whether to copy them.
    fn ask_duplicate(&mut self, id: &str) {
        let has_kids = self.col.as_ref().is_some_and(|c| !c.children(Some(id)).is_empty());
        if has_kids {
            self.dup_dialog = Some(id.to_string());
        } else {
            self.duplicate(id, false);
        }
    }

    /// A copy of `id` beside it (same parent), with a new id and "(copy)" after the title,
    /// and with `children`, a copy of everything below it too. The same lines from the same
    /// start play the same, so the copies keep the originals' hashes and start clean.
    fn duplicate(&mut self, id: &str, children: bool) {
        let Some(col) = self.col.as_ref() else { return };
        if !col.replays.contains_key(id) {
            return;
        }
        let olds = if children { subtree(col, id) } else { vec![id.to_string()] };
        let map: HashMap<String, String> = olds.iter().map(|o| (o.clone(), Self::new_id())).collect();
        let copies: Vec<decomp_dj::format::Replay> = olds
            .iter()
            .map(|o| {
                let mut r = col.replays[o].clone();
                r.set("id", &map[o]);
                if o == id {
                    r.set("title", &format!("{} (copy)", r.title()));
                } else if let Some(p) = r.parent().and_then(|p| map.get(p)).cloned() {
                    r.set("parent", &p);
                }
                r
            })
            .collect();
        let new = map[id].clone();
        let new_ids: Vec<&str> = olds.iter().map(|o| map[o].as_str()).collect();
        if !self.remember("Duplicate", &new_ids) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        for r in copies {
            let rid = r.get("id").unwrap_or_default().to_string();
            col.replays.insert(rid.clone(), r);
            self.edited.insert(rid);
        }
        if let Some(p) = col.replays[&new].parent().map(String::from) {
            self.tree_closed.remove(&p);
        }
        let n = olds.len();
        self.info(if n == 1 {
            "Duplicated. The copy has the same inputs and hashes; its children weren't copied.".to_string()
        } else {
            format!("Duplicated {n} segments: the copy and the {} below it, with the same inputs and hashes.", n - 1)
        });
        self.select(&new);
    }

    /// Adds an empty child segment that starts where `parent` ends, and selects it.
    fn new_child(&mut self, parent: &str) {
        let id = Self::new_id();
        self.remember("New segment", &[&id]);
        let Some(col) = &mut self.col else { return };
        let r = decomp_dj::format::Replay::new(&id, Some(parent), "New segment", 600);
        col.replays.insert(id.clone(), r);
        self.edited.insert(id.clone());
        self.tree_closed.remove(parent);
        self.info("Added a 600-frame segment. Change its title and length with Edit > Properties….");
        self.select(&id);
        self.open_node_dialog(&id);
    }

    /// Splits the selected segment at the playhead: frames before it stay in this segment,
    /// the rest move to a new child that inherits this segment's children. With `branch`,
    /// also adds an empty sibling of that child, so a second path starts at the playhead.
    fn split_here(&mut self, branch: bool) {
        if let Err(e) = self.try_split(branch) {
            self.error(e);
        }
    }

    fn try_split(&mut self, branch: bool) -> Result<()> {
        let pb = self.pb.as_ref().ok_or_else(|| anyhow!("select a segment first"))?;
        let id = pb.id.clone();
        // The playhead shows frame f; the split goes right after it, so frame f stays upstream.
        let f = pb.shown().unwrap_or(0) + 1;
        if f >= pb.frames {
            if branch {
                self.new_child(&id);
                return Ok(());
            }
            return Err(anyhow!("the playhead is on the last frame: use New child at end instead"));
        }
        self.playing = false;
        // State hash at the start of frame f: run exactly f frames.
        self.seek(f - 1);
        let (Some(col), Some(s)) = (&mut self.col, &mut self.session) else { return Ok(()) };
        let h = s.state_hash(col)?;
        let up_id = Self::new_id();
        let b = Self::new_id();
        {
            let mut ids = vec![id.as_str(), up_id.as_str()];
            if branch {
                ids.push(b.as_str());
            }
            if !self.remember(if branch { "Branch" } else { "Split" }, &ids) {
                return Ok(());
            }
        }
        let (Some(col), Some(_)) = (&mut self.col, &mut self.session) else { return Ok(()) };
        let (mut up, mut down) = col.replays[&id].split_at(f, &up_id)?;
        if let Some((_, want)) = col.replays[&id].checkpoints()?.into_iter().find(|(cf, _)| *cf == f) {
            if want != h {
                self.message = Some((format!("Note: the recorded checkpoint at frame {f} does not match this playback"), true));
            }
        }
        // The cut gets a fresh state hash. The second half keeps the segment's id, children and
        // old end hash, so it and everything below are assumed clean.
        up.set("end-hash", &h);
        down.set("start-hash", &h);
        col.replays.insert(up_id.clone(), up);
        col.replays.insert(id.clone(), down);
        self.edited.insert(id.clone());
        self.edited.insert(up_id.clone());
        self.roll.invalidate();
        if branch {
            let r = decomp_dj::format::Replay::new(&b, Some(&up_id), "New branch", 600);
            col.replays.insert(b.clone(), r);
            self.edited.insert(b.clone());
            self.info(format!("Split at frame {f}: the first {f} frames are a new parent segment, and a new branch starts beside the rest."));
            self.select(&b);
            self.open_node_dialog(&b);
        } else {
            self.info(format!("Split at frame {f}: the first {f} frames are now a new parent segment."));
            self.select(&up_id);
            self.seek(f - 1);
        }
        Ok(())
    }

    fn open_node_dialog(&mut self, id: &str) {
        let Some(col) = &self.col else { return };
        let r = &col.replays[id];
        self.node_dialog = Some(NodeDialog {
            id: id.to_string(),
            title: r.title().to_string(),
            frames: r.frames().unwrap_or(0),
            tags: r.get("tags").unwrap_or_default().to_string(),
            root: r.parent().is_none(),
            sram: StartFile::of(r, "sram"),
            snapshot: StartFile::of(r, "snapshot"),
        });
    }

    fn apply_node_dialog(&mut self, d: &NodeDialog) -> Result<()> {
        let Some(col) = &mut self.col else { return Ok(()) };
        let r = col.replays.get_mut(&d.id).ok_or_else(|| anyhow!("segment is gone"))?;
        let old_frames = r.frames()?;
        if let Some(last) = r.lines.last() {
            if d.frames <= last.frame {
                return Err(anyhow!("frame {} has lines; delete them first or keep at least {} frames", last.frame, last.frame + 1));
            }
        }
        if d.frames == 0 {
            return Err(anyhow!("a segment needs at least one frame"));
        }
        // What a root segment starts from: a picked file becomes a blob on Apply.
        let starts = [("snapshot", &d.snapshot), ("sram", &d.sram)];
        let changed: Vec<(&str, &StartFile, Option<String>)> =
            starts.iter().filter(|_| d.root).map(|(f, sf)| (*f, *sf, sf.blob_after())).filter(|(f, _, b)| b.as_deref() != r.get(f)).collect();
        let start_changed = !changed.is_empty();
        let mut seen = HashSet::new();
        let tags = d.tags.split_whitespace().filter(|t| seen.insert(t.to_lowercase())).collect::<Vec<_>>().join(" ");
        let tags_changed = tags != r.get("tags").unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        if d.title.trim() == r.title() && d.frames == old_frames && !start_changed && !tags_changed {
            return Ok(());
        }
        if !self.remember("Segment properties", &[&d.id]) {
            return Ok(());
        }
        let Some(col) = &mut self.col else { return Ok(()) };
        for (field, sf, _) in &changed {
            if let Some((path, data)) = &sf.pick {
                col.write_blob(data, field).map_err(|e| anyhow!("copying {} into blobs/: {e:#}", path.display()))?;
            }
        }
        let r = col.replays.get_mut(&d.id).ok_or_else(|| anyhow!("segment is gone"))?;
        r.set("title", d.title.trim());
        if tags_changed {
            // Tags change no state: nothing to replay.
            if tags.is_empty() {
                r.remove_all("tags");
            } else {
                r.set("tags", &tags);
            }
        }
        self.edited.insert(d.id.clone());
        if start_changed {
            // A new start state: nothing recorded for this segment holds any more, from its
            // start hash on, and everything below turns dirty.
            for (field, _, blob) in &changed {
                match blob {
                    Some(b) => r.set(field, b),
                    None => r.remove_all(field),
                }
            }
            r.remove_all("start-hash");
            r.remove_all("checkpoint");
            r.remove_all("end-hash");
            let e = self.dirty_from.entry(d.id.clone()).or_insert(0);
            *e = 0;
            self.roll.invalidate();
            self.coverage_stale();
            if self.selected.as_deref() == Some(d.id.as_str()) {
                self.select(&d.id.clone());
            }
            let what: Vec<&str> = changed.iter().map(|(f, _, b)| match (*f, b.is_some()) {
                ("sram", true) => "save file set",
                ("sram", false) => "save file cleared",
                (_, true) => "snapshot set",
                (_, false) => "snapshot cleared",
            }).collect();
            let what = what.join(", ");
            let what = what[..1].to_uppercase() + &what[1..];
            self.info(format!("{what}: the segment starts differently now. It and everything below are dirty until cleaned."));
            if d.frames == old_frames {
                return Ok(());
            }
            let Some(col) = &mut self.col else { return Ok(()) };
            let r = col.replays.get_mut(&d.id).ok_or_else(|| anyhow!("segment is gone"))?;
            r.set("frames", &d.frames.to_string());
            self.roll.invalidate();
            if self.selected.as_deref() == Some(d.id.as_str()) {
                self.select(&d.id.clone());
            }
            return Ok(());
        }
        if d.frames != old_frames {
            r.set("frames", &d.frames.to_string());
            self.invalidate_from(&d.id.clone(), d.frames.min(old_frames));
            if self.selected.as_deref() == Some(d.id.as_str()) {
                let cur = self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0);
                self.select(&d.id.clone());
                self.seek(cur.min(d.frames - 1));
            }
        }
        self.roll.invalidate();
        Ok(())
    }

    fn open_frame_dialog(&mut self, frame: u64, new_text: &str) {
        self.frame_dialog = Some(FrameDialog { frame, rows: vec![], new_text: new_text.to_string(), focus: true });
        self.refresh_frame_dialog();
    }

    fn refresh_frame_dialog(&mut self) {
        let (Some(col), Some(id), Some(d)) = (&self.col, &self.selected, &mut self.frame_dialog) else { return };
        let r = &col.replays[id];
        d.rows = r.lines.iter().filter(|l| l.frame == d.frame && !matches!(l.command, Command::Input { .. })).filter_map(|l| r.line_command_text(l.line_no).map(|t| (l.line_no, t))).collect();
    }

    /// Changes a non-input line on the selected segment: replace (Some) or delete (None) an
    /// existing line, or add a new one when `line_no` is None.
    fn edit_line(&mut self, frame: u64, line_no: Option<usize>, text: Option<&str>) {
        let (Some(col), Some(id)) = (&mut self.col, self.selected.clone()) else { return };
        let r = col.replays.get(&id).unwrap();
        // Events and screenshots don't touch the game; everything else may.
        let harmless = |t: &str| {
            let t = t.trim_start().to_ascii_uppercase();
            t.starts_with("[EVENT") || t.starts_with("[SCREENSHOT")
        };
        let old_text = line_no.and_then(|n| r.line_command_text(n));
        if let (Some(o), Some(t)) = (&old_text, text) {
            if o.trim() == t.trim() {
                return;
            }
        }
        if line_no.is_none() && text.is_none() {
            return;
        }
        if !self.remember(if text.is_none() { "Delete line" } else if line_no.is_none() { "Add line" } else { "Edit line" }, &[&id]) {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let r = col.replays.get_mut(&id).unwrap();
        let res = match (line_no, text) {
            (Some(n), t) => r.replace_line(n, t),
            (None, Some(t)) => r.insert_line(frame, t),
            (None, None) => Ok(()),
        };
        if let Err(e) = res {
            self.error(e);
            return;
        }
        self.edited.insert(id.clone());
        self.roll.invalidate();
        let changes_state = !(old_text.as_deref().is_none_or(harmless) && text.is_none_or(harmless));
        if changes_state {
            self.invalidate_from(&id, frame);
        }
        self.refresh_frame_dialog();
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(msg) = &self.compare_dialog {
            let mut ok = false;
            let mut open = true;
            egui::Window::new("Comparison core").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(msg);
                ui.label(RichText::new("Only the baseline is shown. The entries are [[compare]] in config.toml, or .local/local.toml for this machine.").weak());
                ui.add_space(4.0);
                ok = ui.button("OK").clicked();
            });
            if ok || !open {
                self.compare_dialog = None;
            }
        }
        if !self.recovery_offer.is_empty() {
            self.recovery_window(ctx);
        }
        if let Some(what) = self.discard_dialog.take() {
            let n = self.unsaved_edits();
            let mut open = true;
            let (mut save, mut discard, mut cancel) = (false, false, false);
            egui::Window::new("Unsaved edits").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("{n} segment{} {} unsaved edits.", if n == 1 { "" } else { "s" }, if n == 1 { "has" } else { "have" })).strong());
                ui.label(format!("Save them before you {}, or discard them? Undo can't bring discarded edits back.", what.verb()));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    save = ui.button(format!("Save and {}", what.button().to_lowercase())).clicked();
                    discard = ui.button(format!("Discard and {}", what.button().to_lowercase())).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if n == 0 || discard {
                self.discard_go(what);
            } else if save {
                // Saving may ask first (segments it would leave dirty): then the save dialog takes
                // over, and the person repeats Open, Reload or Quit once it's done.
                self.save();
                if self.unsaved_edits() == 0 {
                    self.discard_go(what);
                } else if self.save_dialog.is_some() {
                    self.info(format!("Saving first: {} again once it's saved.", what.button()));
                }
            } else if !cancel && open {
                self.discard_dialog = Some(what);
            }
        }
        // One confirmation for Edit > Delete and the tree's right-click Delete.
        if let Some(ask) = self.save_dialog.take() {
            let mut open = true;
            let (mut clean, mut save, mut close) = (false, false, false);
            egui::Window::new("Save").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                let s = |n: usize| if n == 1 { "" } else { "s" };
                match &ask {
                    SaveAsk::Dirty(n) => {
                        ui.label(RichText::new(format!("{n} segment{} would be saved dirty.", s(*n))).strong());
                        ui.label("Dirty segments are unrecorded (no recorded ending yet) or stale (they start where their parent no longer ends). Cleaning plays them and records their hashes; saved dirty, they stay dirty until cleaned and saved again.");
                    }
                    SaveAsk::AfterClean { changed, dirty, failed } => {
                        if let Some(e) = failed {
                            ui.label(RichText::new(format!("Cleaning didn't finish: {e}")).strong().color(Color32::from_rgb(240, 160, 60)));
                        }
                        if *changed > 0 {
                            ui.label(RichText::new(format!("{changed} ending{} changed (marked ! in the tree).", s(*changed))).strong());
                            ui.label("Review them before saving: saving clears the marks.");
                        }
                        if *dirty > 0 {
                            ui.label(format!("{dirty} segment{} would still be saved dirty.", s(*dirty)));
                        }
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| match &ask {
                    SaveAsk::Dirty(_) => {
                        clean = ui.button("Clean, then save").clicked();
                        save = ui.button("Save anyway").clicked();
                        close = ui.button("Cancel").clicked();
                    }
                    SaveAsk::AfterClean { dirty, failed, .. } => {
                        let label = if *dirty > 0 || failed.is_some() { "Save anyway" } else { "Save" };
                        save = ui.button(label).clicked();
                        close = ui.button("Review first").clicked();
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
            if clean {
                self.save_after_clean = true;
                self.start_verify(None, Clean::Unclean);
                if self.hydrating.is_none() {
                    // Nothing could be cleaned (no database, say): leave it to the person.
                    self.save_after_clean = false;
                }
            } else if save {
                self.save_now();
            } else if !close && open {
                self.save_dialog = Some(ask);
            }
        }
        if let Some(id) = self.dup_dialog.clone() {
            let info = self.col.as_ref().filter(|c| c.replays.contains_key(&id)).map(|c| {
                let below = &subtree(c, &id)[1..];
                let frames: u64 = below.iter().filter_map(|i| c.replays.get(i).and_then(|r| r.frames().ok())).sum();
                (below.len(), frames)
            });
            match info {
                None => self.dup_dialog = None,
                Some((below, frames)) => {
                    let mut open = true;
                    let (mut one, mut all, mut cancel) = (false, false, false);
                    egui::Window::new("Duplicate segment").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                        ui.label(format!("Duplicate {below} {}? ({frames} frames)", if below == 1 { "child" } else { "children" }));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            all = ui.button("With children").on_hover_text("Copy the whole branch: every segment below this one comes along, under the copy").clicked();
                            one = ui.button("Just this segment").on_hover_text("Copy only this segment; the copy has no children").clicked();
                            cancel = ui.button("Cancel").clicked();
                        });
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        }
                    });
                    if one || all {
                        self.dup_dialog = None;
                        self.duplicate(&id, all);
                    } else if cancel || !open {
                        self.dup_dialog = None;
                    }
                }
            }
        }
        let pending = self.delete_dialog.clone().and_then(|id| self.col.as_ref().filter(|c| c.replays.contains_key(&id)).map(|c| (id, c)));
        if pending.is_none() {
            self.delete_dialog = None;
        }
        if let Some((id, col)) = pending {
            let r = &col.replays[&id];
            let ids = subtree(col, &id);
            let frames: u64 = ids.iter().map(|i| col.replays[i].frames().unwrap_or(0)).sum();
            let title = r.title().to_string();
            let mut open = true;
            let mut confirm = false;
            let mut keep = false;
            let mut cancel = false;
            let below = ids.len() - 1;
            egui::Window::new("Delete segment").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("Delete \"{title}\"?")).strong());
                if below > 0 {
                    ui.label(RichText::new(format!("This also deletes the {below} segment{} below it.", if below == 1 { "" } else { "s" })).color(Color32::from_rgb(240, 160, 60)));
                }
                ui.label(format!("{} segment{}, {frames} frames in all. Undo brings them back; the files are removed when you save.", ids.len(), if ids.len() == 1 { "" } else { "s" }));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new(if ids.len() == 1 { "Delete".to_string() } else { format!("Delete {} segments", ids.len()) }).color(Color32::from_rgb(240, 110, 100))).clicked() {
                        confirm = true;
                    }
                    if below > 0
                        && ui
                            .button("Delete, keep children")
                            .on_hover_text("Delete only this segment: its children move up to its parent (or become root segments) and turn dirty")
                            .clicked()
                    {
                        keep = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if confirm {
                self.delete_dialog = None;
                self.delete_nodes(&id, ids);
            } else if keep {
                self.delete_dialog = None;
                self.delete_keep_children(&id);
            } else if cancel || !open {
                self.delete_dialog = None;
            }
        }
        self.move_dialog_ui(ctx);
        if let Some((top, mut markers)) = self.chain_dialog.take() {
            let run = self.only_child_run(&top);
            let col = self.col.as_ref();
            let title = col_title(col, &top);
            let total: u64 = std::iter::once(&top).chain(run.iter()).filter_map(|id| col.and_then(|c| c.replays.get(id)).and_then(|r| r.frames().ok())).sum();
            let names: Vec<String> = run.iter().map(|id| col_title(col, id)).collect();
            let mut open = !run.is_empty();
            let (mut go, mut cancel) = (false, false);
            egui::Window::new("Join chain").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("Join {} segment{} into \"{title}\"?", run.len(), if run.len() == 1 { "" } else { "s" })).strong());
                ui.label(format!("\"{title}\" keeps its name and grows to {total} frames, with the same inputs: {}. One undo splits them all again.", names.join(" › ")));
                ui.add_space(6.0);
                ui.checkbox(&mut markers, "Leave a marker with each segment's title where it began");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    go = ui.button("Join").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if go {
                if let Err(e) = self.join_chain(&top, markers) {
                    self.error(e);
                }
            } else if !cancel && open {
                self.chain_dialog = Some((top, markers));
            }
        }
        let pending = self.join_dialog.take().and_then(|d| {
            let col = self.col.as_ref()?;
            let r = col.replays.get(&d.0)?;
            let p = col.replays.get(r.parent()?)?;
            Some((d, r.title().to_string(), p.title().to_string(), p.frames().unwrap_or(0), r.frames().unwrap_or(0)))
        });
        if let Some(((id, mut mark, mut text), title, ptitle, pf, cf)) = pending {
            let mut open = true;
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Join with parent").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.label(RichText::new(format!("Join \"{title}\" into \"{ptitle}\"?")).strong());
                ui.label(format!("\"{ptitle}\" keeps its name and grows from {pf} to {} frames, with the same inputs. Undo splits them again.", pf + cf));
                ui.add_space(6.0);
                ui.checkbox(&mut mark, format!("Leave a marker at frame {pf}, where the split was"));
                let edit = ui.add_enabled(mark, egui::TextEdit::singleline(&mut text).id_salt("join_marker").desired_width(320.0).hint_text("what happens here"));
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirm = true;
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Join").clicked() {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
            });
            if confirm {
                if let Err(e) = self.try_join(&id, mark.then_some(text.as_str())) {
                    self.error(e);
                }
            } else if !cancel && open {
                self.join_dialog = Some((id, mark, text));
            }
        }
        if let Some(mut d) = self.node_dialog.take() {
            let mut open = true;
            let mut apply = false;
            egui::Window::new("Segment properties").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                egui::Grid::new("node_props").num_columns(2).show(ui, |ui| {
                    ui.label("Title");
                    ui.add(egui::TextEdit::singleline(&mut d.title).desired_width(320.0));
                    ui.end_row();
                    ui.label("Tags");
                    ui.add(egui::TextEdit::singleline(&mut d.tags).hint_text("space-separated, e.g. chapter-2 town").desired_width(320.0))
                        .on_hover_text("Free tags, separated by spaces. Filter the tree by one with #tag in the filter box, or the tag menu beside it");
                    ui.end_row();
                    ui.label("Frames");
                    ui.add(egui::DragValue::new(&mut d.frames).range(1..=10_000_000).speed(10.0));
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new(format!("{:.1} s at {:.2} fps", d.frames as f64 / self.fps, self.fps)).weak());
                    ui.end_row();
                    if d.root {
                        let rows = [
                            ("Snapshot", "none (power-on)", "A save state of this core to start from instead of power-on. It is copied into blobs/.", "Choose a snapshot (save state)", &mut d.snapshot),
                            ("Save file", "none (no save data)", "Battery-backed save RAM to load before frame 0 (an .srm, or the core's save file), after the snapshot if there is one. It is copied into blobs/.", "Choose a save file (SRAM)", &mut d.sram),
                        ];
                        for (label, none, tip, pick_title, sf) in rows {
                            ui.label(label);
                            ui.horizontal(|ui| {
                                let shown = match (&sf.pick, &sf.blob) {
                                    (Some((p, data)), _) => format!("{} ({} bytes, not applied yet)", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), data.len()),
                                    (None, Some(b)) => format!("blob {}…", &b[..b.len().min(12)]),
                                    (None, None) => none.into(),
                                };
                                ui.label(shown);
                                if ui.button("Choose…").on_hover_text(tip).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().set_title(pick_title).pick_file() {
                                        match std::fs::read(&f) {
                                            Ok(data) => sf.pick = Some((f, data)),
                                            Err(e) => self.message = Some((format!("Reading {}: {e}", f.display()), true)),
                                        }
                                    }
                                }
                                if ui.add_enabled(sf.blob.is_some() || sf.pick.is_some(), egui::Button::new("Clear")).clicked() {
                                    sf.blob = None;
                                    sf.pick = None;
                                }
                            });
                            ui.end_row();
                        }
                    }
                });
                if d.root {
                    ui.label(RichText::new("Changing the snapshot or save file changes how the segment starts: it and everything below turn dirty.").weak().small());
                }
                ui.label(RichText::new(format!("id {}", d.id)).weak().small());
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                });
            });
            if apply {
                match self.apply_node_dialog(&d) {
                    Ok(()) => open = false,
                    Err(e) => self.error(e),
                }
            }
            if open {
                self.node_dialog = Some(d);
            }
        }
        if let Some(mut d) = self.frame_dialog.take() {
            let mut open = true;
            let mut change: Option<(Option<usize>, Option<String>)> = None;
            let title = format!("Frame {}", d.frame);
            egui::Window::new(title).id(egui::Id::unique("frame_dialog")).open(&mut open).collapsible(false).default_width(460.0).show(ctx, |ui| {
                ui.label(RichText::new("Events, RAM writes and other lines on this frame. Edit the text after the frame number.").weak());
                ui.add_space(4.0);
                if d.rows.is_empty() {
                    ui.label(RichText::new("No lines on this frame yet.").italics());
                }
                for (n, text) in &mut d.rows {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(text).desired_width(330.0).font(egui::TextStyle::Monospace));
                        if ui.button("Apply").clicked() {
                            change = Some((Some(*n), Some(text.clone())));
                        }
                        if ui.button("Delete").clicked() {
                            change = Some((Some(*n), None));
                        }
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("New:");
                    if ui.small_button("event").clicked() {
                        d.new_text = "[EVENT] ".into();
                    }
                    if ui.small_button("RAM write").clicked() {
                        d.new_text = "[WRITE] RAM 0x0000 00".into();
                    }
                });
                ui.horizontal(|ui| {
                    let te = ui.add(egui::TextEdit::singleline(&mut d.new_text).desired_width(330.0).font(egui::TextStyle::Monospace).hint_text("[EVENT] Boss appears"));
                    if d.focus {
                        te.request_focus();
                        d.focus = false;
                    }
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Add").clicked() || enter) && !d.new_text.trim().is_empty() {
                        change = Some((None, Some(d.new_text.clone())));
                    }
                });
                ui.label(RichText::new("A write takes any number of bytes: [WRITE] RAM 0x0300 01 02 03 …").weak().small());
            });
            let frame = d.frame;
            let added = matches!(change, Some((None, Some(_))));
            self.frame_dialog = open.then_some(d);
            if let Some((n, t)) = change {
                self.edit_line(frame, n, t.as_deref());
                if added && !self.message.as_ref().is_some_and(|m| m.1) {
                    if let Some(d) = &mut self.frame_dialog {
                        d.new_text = "[EVENT] ".into();
                        d.focus = true;
                    }
                }
            }
        }
    }

    // ---- UI ----

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        // Menus ignore clicks inside them (CloseOnClickOutside) and are closed below on any
        // click that wasn't on a checkbox, so several boxes can be ticked in a row.
        let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let mut bar: Vec<egui::InnerResponse<Option<()>>> = vec![];
        egui::MenuBar::new().config(config).ui(ui, |ui| {
            bar.push(ui.menu_button("File", |ui| {
                if ui.button("Open project…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().set_title("Open a project folder").pick_folder() {
                        self.discard_guard(Discard::Open(dir));
                    }
                }
                let n = self.unsaved().len();
                if ui.add_enabled(n > 0, egui::Button::new(format!("Save ({n} unsaved)")).shortcut_text("Ctrl+S")).on_hover_text("Write every changed segment, and move drafts into segments/").clicked() {
                    self.save();
                }
                let sel = self.selected.clone().unwrap_or_default();
                let blocked = if self.selected.is_some() { self.save_one_blocked(&sel) } else { Some("No segment selected") };
                if ui.add_enabled(blocked.is_none(), egui::Button::new("Save selected segment")).on_hover_text("Write only this segment's file; other unsaved edits stay unsaved").on_disabled_hover_text(blocked.unwrap_or("")).clicked() {
                    self.save_one(&sel);
                }
                let blocked = if self.selected.is_some() { self.revert_blocked(&sel) } else { Some("No segment selected") };
                if ui.add_enabled(blocked.is_none(), egui::Button::new("Revert selected segment")).on_hover_text("Drop this segment's unsaved changes and put back its saved file; other edits are kept. Undo brings the changes back").on_disabled_hover_text(blocked.unwrap_or("")).clicked() {
                    self.revert_one(&sel);
                }
                if ui.add_enabled(self.col.is_some(), egui::Button::new("Reload all from disk")).on_hover_text("Read every segment's file again, dropping the unsaved edits of every segment (it asks first when there are any). To drop one segment's edits, use Revert selected segment").clicked() {
                    self.discard_guard(Discard::Reload);
                }
                ui.separator();
                ui.add_enabled(false, egui::Button::new("Options…")).on_disabled_hover_text("Coming: your GUI preferences from .local/local.toml in one window");
                if ui.button("Keyboard mapping…").on_hover_text("Keys for recording inputs from the keyboard").clicked() {
                    self.mapping = Some(record::MappingDialog::new(&self.bindings));
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    self.discard_guard(Discard::Quit);
                }
            }));
            bar.push(ui.menu_button("Edit", |ui| {
                use egui::{Key, KeyboardShortcut, Modifiers};
                let ctx = ui.ctx().clone();
                let sc = |m: Modifiers, k: Key| ctx.format_shortcut(&KeyboardShortcut::new(m, k));
                let undo_label = self.undo.last().map_or("Undo".to_string(), |s| format!("Undo {}", s.label.to_lowercase()));
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new(undo_label).shortcut_text(sc(Modifiers::COMMAND, Key::Z))).clicked() {
                    self.undo();
                }
                let redo_label = self.redo.last().map_or("Redo".to_string(), |s| format!("Redo {}", s.label.to_lowercase()));
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new(redo_label).shortcut_text(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))).clicked() {
                    self.redo();
                }
                ui.separator();
                let has_sel = !self.roll.selected_notes().is_empty();
                if ui.add_enabled(has_sel, egui::Button::new("Cut").shortcut_text(sc(Modifiers::COMMAND, Key::X))).clicked() {
                    self.cut();
                }
                if ui.add_enabled(has_sel, egui::Button::new("Copy").shortcut_text(sc(Modifiers::COMMAND, Key::C))).clicked() {
                    self.copy();
                }
                if ui.add_enabled(self.clip.is_some() && self.pb.is_some(), egui::Button::new("Paste at play cursor").shortcut_text(sc(Modifiers::COMMAND, Key::V))).clicked() {
                    self.paste();
                }
                ui.separator();
                if ui.add_enabled(self.selected.is_some(), egui::Button::new("Select all").shortcut_text(sc(Modifiers::COMMAND, Key::A))).on_hover_text("Select every press of the selected segment in the piano roll").clicked() {
                    self.roll.select_all();
                }
                ui.checkbox(&mut self.roll.draw, "Drag to draw inputs").on_hover_text(
                    "In the piano roll, a drag on an empty cell draws a press instead of selecting an area. Hold Ctrl/Cmd for the other one.",
                );
                ui.separator();
                let sel = self.selected.clone();
                let has = sel.is_some() && self.pb.is_some();
                if ui.add_enabled(has, egui::Button::new("New child at end")).on_hover_text("A new empty segment that starts where this one ends").clicked() {
                    self.new_child(sel.as_deref().unwrap());
                }
                if ui.add_enabled(self.col.is_some(), egui::Button::new("New root segment")).on_hover_text(NEW_ROOT_TIP).clicked() {
                    self.new_root();
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new("Duplicate")).on_hover_text(DUP_TIP).clicked() {
                    self.ask_duplicate(sel.as_deref().unwrap());
                }
                if ui.add_enabled(has, egui::Button::new("Branch at playhead")).on_hover_text("Split this segment after the playhead and start a second path there").clicked() {
                    self.split_here(true);
                }
                if ui.add_enabled(has, egui::Button::new("Split at playhead")).on_hover_text("Cut this segment in two after the playhead; its children move to the second half").clicked() {
                    self.split_here(false);
                }
                ui.separator();
                if ui.add_enabled(has, egui::Button::new("Add event at playhead…").shortcut_text("E")).clicked() {
                    let f = self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0);
                    self.open_frame_dialog(f, "[EVENT] ");
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new("Properties…")).clicked() {
                    self.open_node_dialog(sel.as_deref().unwrap());
                }
                let joinable = sel.as_deref().is_some_and(|s| self.can_join(s));
                if ui.add_enabled(joinable, egui::Button::new("Join with parent")).on_hover_text(JOIN_TIP).on_disabled_hover_text("Only for a segment whose parent has no other children").clicked() {
                    self.join_with_parent(&sel.clone().unwrap());
                }
                let only = sel.as_deref().and_then(|s| self.only_child(s));
                if ui.add_enabled(only.is_some(), egui::Button::new("Join with only child")).on_hover_text(JOIN_CHILD_TIP).on_disabled_hover_text("Only for a segment with exactly one child").clicked() {
                    self.join_with_parent(&only.unwrap());
                }
                let chain = sel.as_deref().is_some_and(|s| self.only_child_run(s).len() >= 2);
                if ui.add_enabled(chain, egui::Button::new("Join chain…")).on_hover_text(JOIN_CHAIN_TIP).on_disabled_hover_text("Only where at least two only-children follow in a row").clicked() {
                    self.chain_dialog = Some((sel.clone().unwrap(), true));
                }
                ui.separator();
                if ui.add_enabled(sel.is_some(), egui::Button::new("Move to…")).on_hover_text(MOVE_TIP).clicked() {
                    self.move_dialog = sel.clone().map(|s| (s, String::new()));
                }
                if ui.add_enabled(sel.is_some(), egui::Button::new("Delete…")).on_hover_text("Delete this segment and everything below it, or just it, keeping its children").clicked() {
                    self.delete_dialog = sel.clone();
                }
            }));
            bar.push(ui.menu_button("View", |ui| {
                if ui.button("Zoom in").clicked() {
                    self.roll.zoom(1.5);
                }
                if ui.button("Zoom out").clicked() {
                    self.roll.zoom(1.0 / 1.5);
                }
                ui.checkbox(&mut self.roll.follow, "Follow playhead").keep_open();
                ui.checkbox(&mut self.return_to_edit, "Return to the edit").keep_open().on_hover_text("Any edit pauses playback and shows the earliest frame edited since you last played or moved the playhead. Off: the frame you were on is redrawn.");
                ui.checkbox(&mut self.roll.all_buttons, "Show every joypad button").keep_open();
                ui.separator();
                ui.checkbox(&mut self.show_console, "Debug console")
                    .keep_open()
                    .on_hover_text("A log, below the coverage map, of what dj says while it runs: warnings and notes, the cores' own log messages, what comparison cores and decoder plugins print, and the status bar's messages");
            }));
            bar.push(ui.menu_button("Project", |ui| {
                let db_on = self.session.as_ref().is_some_and(|s| s.cache.is_some());
                let idle = self.hydrating.is_none();
                if ui
                    .add_enabled(db_on && idle, egui::Button::new("Clean all segments"))
                    .on_hover_text("Play, top down, every segment that isn't clean or whose content isn't in the local database yet, and record the hashes it reaches (as reaching a segment's end does). Changed endings are marked ! for you to check. Results are unsaved changes.")
                    .on_disabled_hover_text("Needs the local database")
                    .clicked()
                {
                    self.start_verify(None, Clean::Fill);
                }
                if ui
                    .add_enabled(self.col.is_some() && idle, egui::Button::new("Replay everything"))
                    .on_hover_text("Play every segment again from frame 0 of the first root segment (or of the focused segment), in List mode, as ordinary playback you can pause, watch and speed up: a check that they still end as recorded (after a core update, say). Changed endings are marked !")
                    .clicked()
                {
                    self.replay_everything();
                }
                ui.separator();
                let mut use_db = self.col.as_ref().and_then(|c| c.settings().ok()).and_then(|l| l.database).unwrap_or(true);
                if ui.add_enabled(self.col.is_some() && idle, egui::Checkbox::new(&mut use_db, "Use the local database")).keep_open().on_hover_text("Off: play from power-on and keep snapshots only in memory, as before the database existed").changed() {
                    self.set_database(use_db);
                }
                ui.separator();
                let has_col = self.col.is_some();
                if ui.add_enabled(has_col, egui::Button::new("Set ROM…")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().set_title("Choose the ROM").pick_file() {
                        self.set_local(Some(f), None);
                    }
                }
                if ui.add_enabled(has_col, egui::Button::new("Set core…")).clicked() {
                    if let Some(f) = rfd::FileDialog::new().set_title("Choose the libretro core").add_filter("libretro core", &["so", "dylib", "dll"]).pick_file() {
                        self.set_local(None, Some(f));
                    }
                }
                ui.add_enabled(false, egui::Button::new("Project settings…")).on_disabled_hover_text("Coming: edit the project's config.toml and its local.toml settings here");
            }));
            bar.push(ui.menu_button("Emulation", |ui| {
                let has = self.pb.is_some();
                if ui.add_enabled(has, egui::Button::new(if self.playing { "Pause" } else { "Play" }).shortcut_text("Space")).clicked() {
                    self.toggle_play();
                }
                if ui.add_enabled(has, egui::Button::new("Step forward").shortcut_text("Right arrow")).clicked() {
                    self.step_by(1);
                }
                if ui.add_enabled(has, egui::Button::new("Step back").shortcut_text("Left arrow")).clicked() {
                    self.step_by(-1);
                }
                if ui.add_enabled(has, egui::Button::new("Go to start").shortcut_text("Home")).clicked() {
                    self.playing = false;
                    self.seek(0);
                    self.roll.follow_to(0);
                }
                if ui.add_enabled(has, egui::Button::new("Go to end").shortcut_text("End")).clicked() {
                    self.playing = false;
                    let end = self.pb.as_ref().map_or(0, |p| p.frames.saturating_sub(1));
                    self.seek(end);
                    self.roll.follow_to(end);
                }
                ui.separator();
                // Clicking the submenu's own button leaves the menu open, as egui does by default.
                ui.menu_button(format!("Snapshot memory: {} MB", self.max_memory_mb), |ui| {
                    for mb in [128, 256, 512, 1024, 2048, 4096] {
                        if ui.radio(self.max_memory_mb == mb, format!("{mb} MB")).clicked() {
                            self.set_max_memory(mb);
                        }
                    }
                })
                .response
                .keep_open();
            }));
            bar.push(ui.menu_button("Compare", |ui| self.compare_bar_menu(ui)));
            bar.push(ui.menu_button("MCP", |ui| self.agent_menu(ui)));
        });
        let open = bar.iter().any(|r| r.inner.is_some());
        let on_bar = bar.iter().any(|r| r.response.clicked());
        let clicked = ui.input(|i| i.pointer.any_click());
        let keep = ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(KEEP_MENU_OPEN)).unwrap_or(false));
        if open && clicked && !on_bar && !keep {
            egui::Popup::close_all(ui.ctx());
        }
    }

    /// The ruler band for `id`: from which frame on its hashes aren't known, in the colour of
    /// its tree dot. None when it is clean.
    fn dirty_band(&self, id: &str) -> Option<roll::DirtyBand> {
        let col = self.col.as_ref()?;
        let r = col.replays.get(id)?;
        let status = col.tree_status(id);
        if status == Status::Clean {
            return None;
        }
        let color = status_color(status);
        // While playing after an edit: what hasn't been run again yet.
        if let Some(sf) = self.pb.as_ref().filter(|p| p.id == id).and_then(|p| p.stale_from) {
            return Some(roll::DirtyBand { from: sf, color, hint: format!("Frames from {sf} on haven't been played since the last edit. Play to the end to record the new hashes.") });
        }
        let parent_clean = r.parent().is_none_or(|p| col.tree_status(p) == Status::Clean);
        if r.get("end-hash").is_some() {
            let hint = if parent_clean {
                "Stale: its parent's end state changed. Play it to the end to record its new hashes (a changed ending gets a !)."
            } else {
                "Stale: a segment above it changed and isn't clean yet. Clean that one first (or use Project > Clean all segments)."
            };
            return Some(roll::DirtyBand { from: 0, color, hint: hint.to_string() });
        }
        let last_cp = r.checkpoints().unwrap_or_default().into_iter().map(|(f, _)| f).max().unwrap_or(0);
        let from = self.dirty_from.get(id).copied().unwrap_or(last_cp);
        let hint = if parent_clean {
            format!("Unrecorded from frame {from} on: its hashes aren't recorded. Play to the end to record them.")
        } else {
            format!("Unrecorded from frame {from} on, and its parent isn't clean: clean that first, or use Project > Clean all segments.")
        };
        Some(roll::DirtyBand { from, color, hint })
    }

    fn tree(&mut self, ui: &mut egui::Ui) {
        // Save and Revert on right-click: why each is off, for the segments with unsaved edits.
        let one_why: HashMap<String, (Option<&'static str>, Option<&'static str>)> =
            self.unsaved().into_iter().map(|id| { let w = (self.save_one_blocked(&id), self.revert_blocked(&id)); (id, w) }).collect();
        let none_why = (Some("No unsaved changes in this segment"), Some("No unsaved changes in this segment"));
        let Some(col) = &self.col else {
            ui.add_space(8.0);
            ui.label("No project open.");
            if ui.button("Open project…").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.open_collection(&dir);
                }
            }
            return;
        };
        ui.add_space(4.0);
        ui.heading(&col.config.name);
        let (mut clean, mut dirty) = (0, 0);
        for id in col.replays.keys().filter(|id| !col.is_draft(id)) {
            match col.tree_status(id) {
                Status::Clean => clean += 1,
                Status::Dirty => dirty += 1,
            }
        }
        let changed_endings: HashSet<String> = col.replays.keys().filter(|id| self.ending_changed(id)).cloned().collect();
        let owned: HashSet<String> = col.replays.keys().filter(|id| self.agent_owned(id)).cloned().collect();
        let flagged = changed_endings.len();
        ui.horizontal(|ui| {
            for (n, s) in [(clean, Status::Clean), (dirty, Status::Dirty)] {
                status_dot(ui, s);
                ui.label(RichText::new(format!("{n} {}", s.label())).color(status_color(s)).small());
            }
            if flagged > 0 {
                ui.label(RichText::new(format!("! {flagged} changed ending{}", if flagged == 1 { "" } else { "s" })).color(CHANGED).small())
                    .on_hover_text("Unsaved segments whose end hash differs from their saved file. Save to keep them, or undo.");
            }
            if !col.drafts.is_empty() {
                ui.label(RichText::new(format!("{} agent drafts", col.drafts.len())).color(DRAFT).small());
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, name) in [(SideTab::Segments, "Segments"), (SideTab::Markers, "Markers"), (SideTab::Coverage, "Functions")] {
                if ui.selectable_label(self.side_tab == tab, name).clicked() {
                    self.side_tab = tab;
                }
            }
        });
        ui.separator();
        if self.side_tab == SideTab::Coverage {
            self.coverage_tab(ui);
            return;
        }
        if self.side_tab == SideTab::Markers {
            let shown = self.selected.as_deref().zip(self.pb.as_ref().and_then(|b| b.shown()));
            let scope = self.hoist.as_deref().filter(|h| col.replays.contains_key(*h));
            if let Some((id, f)) = marker_list(ui, col, &mut self.marker_filter, shown, scope) {
                if self.selected.as_deref() != Some(id.as_str()) {
                    self.select(&id);
                }
                self.seek(f);
                self.roll.follow_to(f);
            }
            return;
        }
        // Breadcrumbs while the tree is focused on one segment.
        let mut new_hoist: Option<Option<String>> = None;
        let hoist: Option<&str> = self.hoist.as_deref().and_then(|h| col.replays.get_key_value(h)).map(|(k, _)| k.as_str());
        if self.hoist.is_some() && hoist.is_none() {
            new_hoist = Some(None);
        }
        if let Some(h) = hoist {
            let chain = col.chain(h).unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if ui.small_button("All").on_hover_text("Show the whole tree").clicked() {
                    new_hoist = Some(None);
                }
                let n = chain.len();
                // Long paths show the first segment, a menu for the middle, and the last three.
                let (head, hidden, tail) = if n > 5 { (&chain[..1], &chain[1..n - 3], &chain[n - 3..]) } else { (&chain[..0], &chain[..0], &chain[..]) };
                let short = |t: &str| if t.chars().count() > 18 { format!("{}…", t.chars().take(17).collect::<String>()) } else { t.to_string() };
                let crumb = |ui: &mut egui::Ui, id: &str, new_hoist: &mut Option<Option<String>>| {
                    ui.label(RichText::new("›").weak());
                    let t = col.replays[id].title();
                    if id == h {
                        ui.label(RichText::new(short(t)).strong()).on_hover_text(t);
                    } else if ui.link(short(t)).on_hover_text(t).clicked() {
                        *new_hoist = Some(Some(id.to_string()));
                    }
                };
                for id in head {
                    crumb(ui, id, &mut new_hoist);
                }
                if !hidden.is_empty() {
                    ui.label(RichText::new("›").weak());
                    ui.menu_button(format!("… {}", hidden.len()), |ui| {
                        for id in hidden {
                            if ui.button(col.replays[*id].title()).clicked() {
                                new_hoist = Some(Some(id.to_string()));
                            }
                        }
                    });
                }
                for id in tail {
                    crumb(ui, id, &mut new_hoist);
                }
            });
        }
        // Right to left, so the field takes exactly the width left over and never pushes the panel wider.
        let mut fold: Option<bool> = None;
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tip = if self.indent_all { "Indenting every level. Click to keep only children in their parent's column" } else { "Only children stay in their parent's column. Click to indent every level" };
            if nest_button(ui, self.indent_all).on_hover_text(tip).clicked() {
                self.indent_all = !self.indent_all;
            }
            ui.add_space(2.0);
            if fold_button(ui, false).on_hover_text("Expand all").clicked() {
                fold = Some(false);
            }
            if fold_button(ui, true).on_hover_text("Collapse all").clicked() {
                fold = Some(true);
            }
            ui.add_space(2.0);
            let mut all_tags: BTreeMap<String, usize> = BTreeMap::new();
            for r in col.replays.values() {
                // Case-insensitive, like the filter: "Town" and "town" are one tag, counted once a segment.
                let mine: BTreeSet<String> = r.get("tags").unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
                for t in mine {
                    *all_tags.entry(t).or_default() += 1;
                }
            }
            let tag_btn = egui::Button::new(RichText::new("#").strong()).min_size(egui::vec2(20.0, 20.0)).selected(self.filter.trim().starts_with('#') || self.filter_below);
            egui::containers::menu::MenuButton::from_button(tag_btn).ui(ui, |ui| {
                if all_tags.is_empty() {
                    ui.label(RichText::new("No tags yet. Add them in a segment's Properties.").weak());
                }
                for (t, n) in &all_tags {
                    let on = self.filter.trim().eq_ignore_ascii_case(&format!("#{t}"));
                    if ui.selectable_label(on, format!("#{t}  ({n})")).clicked() {
                        self.filter = if on { String::new() } else { format!("#{t}") };
                        ui.close();
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.filter_below, "Show what's below the matches").on_hover_text("While filtering, also show every segment below each match, not only the path down to it");
            }).0.on_hover_text("Filter by tag");
            if !self.filter.is_empty() && ui.small_button("✖").on_hover_text("Clear the filter").clicked() {
                self.filter.clear();
            }
            let w = ui.available_width();
            ui.add(egui::TextEdit::singleline(&mut self.filter).id_salt("tree_filter").hint_text("Filter: title, id, tag or #tag").desired_width(w));
        });
        match fold {
            // Collapse every segment that has children, below the focused segment if there is one,
            // so the roots (or the focused segment's children) stay visible.
            Some(true) => {
                for id in col.replays.keys() {
                    let inside = hoist.is_none_or(|h| h != id && col.chain(id).is_ok_and(|c| c.contains(&h)));
                    if inside && !col.children(Some(id)).is_empty() {
                        self.tree_closed.insert(id.clone());
                    }
                }
            }
            Some(false) => self.tree_closed.clear(),
            None => {}
        }
        let db_on = self.session.as_ref().is_some_and(|s| s.cache.is_some()) && self.hydrating.is_none();
        // While filtering, show the matches and the path down to each of them, ignoring collapsed segments.
        let needle = self.filter.trim().to_lowercase();
        let (matches, shown): (HashSet<&str>, HashSet<&str>) = if needle.is_empty() {
            Default::default()
        } else {
            let mut m = HashSet::new();
            let mut sh = HashSet::new();
            let tag = needle.strip_prefix('#');
            for (id, r) in &col.replays {
                let mut tags = r.get("tags").unwrap_or_default().split_whitespace().map(str::to_lowercase);
                let hit = match tag {
                    Some(t) => tags.any(|x| x == t),
                    None => r.title().to_lowercase().contains(&needle) || id.to_lowercase().starts_with(&needle) || tags.any(|x| x.contains(&needle)),
                };
                let chain = col.chain(id).unwrap_or_default();
                if hit && hoist.is_none_or(|h| chain.contains(&h)) {
                    m.insert(id.as_str());
                    for a in chain {
                        sh.insert(a);
                    }
                }
            }
            if self.filter_below {
                let below: Vec<&str> = m.iter().flat_map(|id| subtree_ids(col, id)).collect();
                sh.extend(below);
            }
            (m, sh)
        };
        let filtering = !needle.is_empty();
        if filtering {
            ui.label(RichText::new(match matches.len() {
                0 => "No matching segments".to_string(),
                1 => "1 match".to_string(),
                n => format!("{n} matches"),
            }).weak().small());
        }
        ui.separator();
        let mut clicked: Option<String> = None;
        let mut double: Option<String> = None;
        let mut rename_done: Option<(String, Option<String>)> = None;
        let mut toggled: Option<String> = None;
        let mut context: Option<(String, &str)> = None;
        // Rows in tree order. An only child continues its parent's column instead of
        // indenting, so indentation counts forks and a long chain stays one column.
        struct Row<'a> {
            id: &'a str,
            indent: usize,
            chained: bool,
        }
        #[allow(clippy::too_many_arguments)]
        fn walk<'a>(col: &'a Collection, id: &'a str, indent: usize, chained: bool, closed: &HashSet<String>, keep: Option<&HashSet<&str>>, indent_all: bool, rows: &mut Vec<Row<'a>>) {
            if keep.is_some_and(|k| !k.contains(id)) {
                return;
            }
            rows.push(Row { id, indent, chained });
            if keep.is_none() && closed.contains(id) {
                return;
            }
            let kids = col.children(Some(id));
            let only = kids.len() == 1 && !indent_all;
            for k in kids {
                walk(col, k, if only { indent } else { indent + 1 }, only, closed, keep, indent_all, rows);
            }
        }
        let mut rows = vec![];
        let roots = match hoist {
            Some(h) => vec![h],
            None => col.children(None),
        };
        for r in roots {
            walk(col, r, 0, false, &self.tree_closed, filtering.then_some(&shown), self.indent_all, &mut rows);
        }
        let path = self.sticky_path();
        let sel_chain: Vec<String> = match &self.selected {
            Some(s) => col.chain(s).map(|c| c.into_iter().map(String::from).collect()).unwrap_or_default(),
            _ => vec![],
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for Row { id, indent, chained } in rows {
                let depth = indent;
                let r = &col.replays[id];
                let has_kids = !col.children(Some(id)).is_empty();
                let closed = self.tree_closed.contains(id);
                let row = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add_space(depth as f32 * 14.0);
                    if has_kids {
                        if ui.add(egui::Button::new(if closed { "⏵" } else { "⏷" }).frame(false).small()).clicked() {
                            toggled = Some(id.to_string());
                        }
                    } else {
                        ui.add_space(14.0);
                    }
                    let st = col.tree_status(id);
                    let tip = match st {
                        Status::Clean => "clean",
                        Status::Dirty if r.get("end-hash").is_none() => "dirty (unrecorded: no end hash yet)",
                        Status::Dirty => "dirty (stale: starts where its parent no longer ends)",
                    };
                    status_dot(ui, st).on_hover_text(tip);
                    if changed_endings.contains(id) {
                        ui.label(RichText::new("!").strong().color(CHANGED)).on_hover_text("Its ending changed since it was last saved. Save to keep it, or undo.");
                    }
                    let mut title = RichText::new(r.title());
                    if filtering && !matches.contains(id) && !(self.filter_below && col.chain(id).is_ok_and(|c| c.iter().any(|a| matches.contains(a)))) {
                        title = title.weak();
                    }
                    let is_draft = col.is_draft(id);
                    // Agent-owned (open in a session): read-only here. Otherwise unsaved: edited
                    // here, or a draft an agent wrote.
                    if owned.contains(id) {
                        title = title.italics().color(AGENT_OWNED);
                    } else if self.edited.contains(id) || is_draft {
                        title = title.italics().color(Color32::from_rgb(240, 210, 90));
                    }
                    let sel = self.selected.as_deref() == Some(id);
                    let resp = match &mut self.renaming {
                        Some((rid, text, focus)) if rid == id => {
                            let w = (ui.available_width() - 50.0).max(80.0);
                            let mut out = egui::TextEdit::singleline(text).id_salt("tree_rename").desired_width(w).show(ui);
                            if *focus {
                                // Start with the whole title selected, as IDEs do.
                                *focus = false;
                                out.response.response.request_focus();
                                let n = text.chars().count();
                                out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                                out.state.store(ui.ctx(), out.response.response.id);
                            } else if out.response.response.lost_focus() {
                                // Enter or clicking away keeps the new title; Escape cancels.
                                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                rename_done = Some((id.to_string(), (!cancel).then(|| text.trim().to_string())));
                            }
                            out.response.response
                        }
                        _ => {
                            // Sticky: the chain above the selection is highlighted, and the sticky
                            // children below it are highlighted more faintly, with faded titles.
                            let above = !sel && sel_chain.iter().any(|c| c == id);
                            let ahead = !sel && !above && path.iter().any(|c| c == id);
                            if above || ahead {
                                let fill = ui.visuals().selection.bg_fill.gamma_multiply(if above { 0.4 } else { 0.18 });
                                let title = if ahead { title.weak() } else { title };
                                ui.add(egui::Button::new(title).fill(fill).stroke(egui::Stroke::NONE))
                            } else {
                                ui.selectable_label(sel, title)
                            }
                        }
                    };
                    let frames = r.frames().unwrap_or(0);
                    ui.label(RichText::new(format!("{frames}f")).weak().small());
                    if self.stored_ids.contains(id) {
                        ui.label(RichText::new("db").small().color(Color32::from_rgb(110, 150, 190))).on_hover_text("Stored: this segment's current content is in the local database");
                    }
                    if let Some(n) = self.cov.unique.get(id) {
                        ui.label(RichText::new(format!("+{n}")).small().color(cov::COVERED))
                            .on_hover_text(format!("{n} instructions that only this segment runs (Functions tab)"));
                    }
                    if r.is_poked() {
                        ui.label(RichText::new("poke").small().color(Color32::from_rgb(200, 120, 220))).on_hover_text("Has [WRITE] lines: memory is changed directly");
                    }
                    if is_draft {
                        let live = self.agent_sessions.iter().find(|s| s.draft == *id).map(|s| s.session.clone());
                        let tag = match &live {
                            Some(s) => format!("agent · {s}"),
                            None => "agent".to_string(),
                        };
                        ui.label(RichText::new(tag).small().color(DRAFT)).on_hover_text(match &live {
                            Some(s) => format!("An agent's draft, open in its session {s}: the agent's, so read-only here, and not saved or cleaned. Right-click to follow it or take it over."),
                            None => "An agent's draft in .local/drafts/: an unsaved segment. File > Save moves it into segments/; Delete removes it.".into(),
                        });
                    }
                    let resp = resp.on_hover_ui(|ui| {
                        ui.label(RichText::new(r.title()).strong());
                        ui.label(format!("id {id}"));
                        ui.label(format!("{frames} frames, status {}", st.label()));
                        let tags: Vec<&str> = r.get("tags").map(|t| t.split_whitespace().collect()).unwrap_or_default();
                        if !tags.is_empty() {
                            ui.label(format!("tags: {}", tags.join(", ")));
                        }
                        if self.edited.contains(id) {
                            ui.label("edited, not saved");
                        }
                        if is_draft {
                            for k in ["draft-note", "draft-condition"] {
                                if let Some(v) = r.get(k) {
                                    ui.label(format!("{}: {v}", k.trim_start_matches("draft-")));
                                }
                            }
                        }
                    });
                    if resp.double_clicked() {
                        double = Some(id.to_string());
                    } else if resp.clicked() {
                        clicked = Some(id.to_string());
                    }
                    resp.context_menu(|ui| {
                        if is_draft {
                            if let Some(s) = self.agent_sessions.iter().find(|s| s.draft == *id) {
                                if ui.button(format!("Follow session {}", s.session)).on_hover_text("Show this draft at the agent's frame as it plays").clicked() {
                                    context = Some((id.to_string(), "follow"));
                                }
                                if owned.contains(id) && ui.button("Take over").on_hover_text("Close the agent's session, keeping the draft, so it is yours to edit and save. The agent is told the next time it uses the session.").clicked() {
                                    context = Some((id.to_string(), "takeover"));
                                }
                                ui.separator();
                            }
                        }
                        if ui.button("Play to here").on_hover_text("Play through to the end of this segment").clicked() {
                            context = Some((id.to_string(), "playto"));
                        }
                        if ui.button("New child at end").clicked() {
                            context = Some((id.to_string(), "child"));
                        }
                        if ui.button("Duplicate").on_hover_text(DUP_TIP).clicked() {
                            context = Some((id.to_string(), "dup"));
                        }
                        if ui.button("Properties…").clicked() {
                            context = Some((id.to_string(), "props"));
                        }
                        let my_tags: BTreeSet<String> = r.get("tags").unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
                        if !my_tags.is_empty() {
                            ui.menu_button("Filter by tag", |ui| {
                                for t in &my_tags {
                                    if ui.button(format!("#{t}")).clicked() {
                                        context = Some((format!("#{t}"), "tagfilter"));
                                    }
                                }
                            });
                        }
                        let (save_why, revert_why) = one_why.get(id as &str).copied().unwrap_or(none_why);
                        if ui.add_enabled(save_why.is_none(), egui::Button::new("Save")).on_hover_text("Write only this segment's file; other unsaved edits stay unsaved").on_disabled_hover_text(save_why.unwrap_or("")).clicked() {
                            context = Some((id.to_string(), "saveone"));
                        }
                        if ui.add_enabled(revert_why.is_none(), egui::Button::new("Revert")).on_hover_text("Drop this segment's unsaved changes and put back its saved file. Undo brings them back").on_disabled_hover_text(revert_why.unwrap_or("")).clicked() {
                            context = Some((id.to_string(), "revert"));
                        }
                        let joinable = r.parent().is_some_and(|p| col.children(Some(p)).len() == 1);
                        if ui.add_enabled(joinable, egui::Button::new("Join with parent")).on_hover_text(JOIN_TIP).on_disabled_hover_text("Only for a segment whose parent has no other children").clicked() {
                            context = Some((id.to_string(), "join"));
                        }
                        let one_child = col.children(Some(id)).len() == 1;
                        if ui.add_enabled(one_child, egui::Button::new("Join with only child")).on_hover_text(JOIN_CHILD_TIP).on_disabled_hover_text("Only for a segment with exactly one child").clicked() {
                            context = Some((id.to_string(), "joinchild"));
                        }
                        let grand_only = one_child && col.children(Some(col.children(Some(id))[0])).len() == 1;
                        if ui.add_enabled(grand_only, egui::Button::new("Join chain…")).on_hover_text(JOIN_CHAIN_TIP).on_disabled_hover_text("Only where at least two only-children follow in a row").clicked() {
                            context = Some((id.to_string(), "joinchain"));
                        }
                        if ui.button("Move to…").on_hover_text(MOVE_TIP).clicked() {
                            context = Some((id.to_string(), "move"));
                        }
                        if ui.button("Delete…").on_hover_text("Delete this segment and everything below it, or just it, keeping its children").clicked() {
                            context = Some((id.to_string(), "delete"));
                        }
                        ui.separator();
                        if ui
                            .add_enabled(db_on, egui::Button::new("Clean this and below"))
                            .on_hover_text("Play what isn't clean or isn't in the local database here and below, and record the hashes it reaches")
                            .on_disabled_hover_text("Needs the local database")
                            .clicked()
                        {
                            context = Some((id.to_string(), "verify"));
                        }
                        ui.separator();
                        if ui.add_enabled(hoist != Some(id), egui::Button::new("Focus here")).on_hover_text("Show only this segment and what is below it").clicked() {
                            context = Some((id.to_string(), "hoist"));
                        }
                        if hoist.is_some() && ui.button("Show whole tree").clicked() {
                            context = Some((id.to_string(), "unhoist"));
                        }
                    });
                });
                if chained {
                    // A short line up to the parent row marks a segment that continues its parent's chain.
                    let rect = row.response.rect;
                    let x = rect.left() + depth as f32 * 14.0 + 7.0;
                    let gap = ui.spacing().item_spacing.y;
                    ui.painter().line_segment([egui::pos2(x, rect.top() - gap - 3.0), egui::pos2(x, rect.top() + 4.0)], egui::Stroke::new(1.5, Color32::from_gray(90)));
                }
            }
            // Below the last segment: a + for a new root segment, and the rest of the panel
            // takes a right-click for the same (not while focused on a branch or filtering).
            if hoist.is_none() && !filtering {
                // Laid out like a root segment's row, where the new one will appear: a + where
                // the status dot goes, then the title in the tree's text size.
                let clicked = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_space(14.0);
                        let (rect, plus) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::click());
                        let label = ui.selectable_label(false, RichText::new("New root segment").weak());
                        let hovered = plus.hovered() || label.hovered();
                        let c = if hovered { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
                        let (m, r) = (rect.center(), 4.0);
                        let stroke = egui::Stroke::new(1.5, c);
                        ui.painter().line_segment([egui::pos2(m.x - r, m.y), egui::pos2(m.x + r, m.y)], stroke);
                        ui.painter().line_segment([egui::pos2(m.x, m.y - r), egui::pos2(m.x, m.y + r)], stroke);
                        let plus = plus.on_hover_text(NEW_ROOT_TIP);
                        let label = label.on_hover_text(NEW_ROOT_TIP);
                        plus.clicked() || label.clicked()
                    })
                    .inner;
                if clicked {
                    context = Some((String::new(), "root"));
                }
                let rest = ui.available_rect_before_wrap();
                if rest.height() > 0.0 {
                    let resp = ui.interact(rest, egui::Id::unique("tree_empty"), egui::Sense::click());
                    resp.context_menu(|ui| {
                        if ui.button("New root segment").on_hover_text(NEW_ROOT_TIP).clicked() {
                            context = Some((String::new(), "root"));
                        }
                    });
                }
            }
        });
        match new_hoist {
            Some(Some(h)) => {
                self.tree_closed.remove(&h);
                self.hoist = Some(h);
            }
            Some(None) => self.hoist = None,
            None => {}
        }
        if let Some(t) = toggled {
            if !self.tree_closed.remove(&t) {
                self.tree_closed.insert(t);
            }
        }
        if let Some(id) = clicked {
            if self.selected.as_deref() != Some(id.as_str()) {
                self.select(&id);
            }
        }
        if let Some(id) = double {
            self.play_node(&id);
        }
        if let Some((id, title)) = rename_done {
            self.renaming = None;
            // The Enter that ended the rename must not start another one.
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let changed = title.as_ref().is_some_and(|t| !t.is_empty() && self.col.as_ref().and_then(|c| c.replays.get(&id)).is_some_and(|r| r.title() != t));
            let changed = changed && self.remember("Rename", &[&id]);
            if let (Some(t), Some(col)) = (title.filter(|_| changed), &mut self.col) {
                if let Some(r) = col.replays.get_mut(&id) {
                    {
                        r.set("title", &t);
                        self.edited.insert(id);
                    }
                }
            }
        }
        match context {
            Some((_, "root")) => self.new_root(),
            Some((id, "joinchain")) => self.chain_dialog = Some((id, true)),
            Some((id, "dup")) => self.ask_duplicate(&id),
            Some((tag, "tagfilter")) => self.filter = tag,
            Some((id, "saveone")) => self.save_one(&id),
            Some((id, "revert")) => self.revert_one(&id),
            Some((id, "child")) => {
                if self.selected.as_deref() != Some(id.as_str()) {
                    self.select(&id);
                }
                self.new_child(&id);
            }
            Some((id, "playto")) => self.play_to(&id),
            Some((id, "hoist")) => {
                self.tree_closed.remove(&id);
                self.hoist = Some(id);
            }
            Some((_, "unhoist")) => self.hoist = None,
            Some((id, "delete")) => self.delete_dialog = Some(id),
            Some((id, "move")) => self.move_dialog = Some((id, String::new())),
            Some((id, "join")) => self.join_with_parent(&id),
            Some((id, "joinchild")) => {
                if let Some(c) = self.only_child(&id) {
                    self.join_with_parent(&c);
                }
            }
            Some((id, "verify")) => self.start_verify(Some(id), Clean::Fill),
            Some((id, "takeover")) => self.take_over(&id),
            Some((id, "follow")) => {
                if let Some(s) = self.agent_sessions.iter().find(|s| s.draft == id) {
                    self.follow = Some((s.pid, s.session.clone()));
                    self.follow_agent();
                }
            }
            Some((id, _)) => self.open_node_dialog(&id),
            None => {}
        }
    }

    // ---- agents ----

    fn agent_menu(&mut self, ui: &mut egui::Ui) {
        let has_col = self.col.is_some();
        let mut on = self.mcp_on;
        if ui
            .add_enabled(has_col, egui::Checkbox::new(&mut on, "Enable MCP"))
            .keep_open()
            .on_hover_text("Work with AI agents running `dj mcp` on this project: show their drafts, follow their sessions, share your view. Saved for this project on this machine.")
            .changed()
        {
            self.set_mcp(on);
        }
        ui.separator();
        let mcp_on = self.mcp_on;
        ui.add_enabled_ui(mcp_on, |ui| self.agent_menu_items(ui, has_col));
    }

    fn agent_menu_items(&mut self, ui: &mut egui::Ui, has_col: bool) {
        if ui.checkbox(&mut self.show_drafts, "Show agent drafts").keep_open().on_hover_text("Drafts in .local/drafts/ appear in the tree as unsaved segments, marked \"agent\"").changed() {
            if self.show_drafts {
                if let Some(col) = &mut self.col {
                    let _ = col.load_drafts();
                }
            } else {
                self.unload_drafts();
            }
        }
        ui.separator();
        ui.label(RichText::new("Follow an agent session").weak().small());
        if self.agent_sessions.is_empty() {
            ui.label(RichText::new("No agent sessions (dj mcp)").weak());
        }
        for s in self.agent_sessions.clone() {
            let title = self.col.as_ref().and_then(|c| c.replays.get(&s.draft)).map(|r| r.title().to_string()).unwrap_or_default();
            let on = self.follow.as_ref() == Some(&(s.pid, s.session.clone()));
            if ui.selectable_label(on, format!("{} · {title} · frame {}", s.session, s.frame)).clicked() {
                self.follow = if on { None } else { Some((s.pid, s.session.clone())) };
                self.follow_agent();
            }
        }
        if ui.add_enabled(self.follow.is_some(), egui::Button::new("Stop following")).clicked() {
            self.follow = None;
        }
        ui.separator();
        if ui
            .add_enabled(has_col, egui::Checkbox::new(&mut self.share_view, "Share my view with agents"))
            .keep_open()
            .on_hover_text("Agents can ask what you are looking at (the gui_view tool) and open a session there to take it from here")
            .changed()
        {
            self.shared = None;
            if !self.share_view {
                if let Some(c) = &self.col {
                    agent::share_view(&c.root, None);
                }
            }
        }
    }

    /// Turns the GUI's MCP side on or off, and remembers it in .local/local.toml.
    fn set_mcp(&mut self, on: bool) {
        self.mcp_on = on;
        if let Some(col) = &self.col {
            let mut local = col.load_local().unwrap_or_default();
            local.mcp.get_or_insert_with(Default::default).gui = Some(on);
            let _ = col.save_local(&local);
        }
        if on {
            if self.show_drafts {
                if let Some(col) = &mut self.col {
                    let _ = col.load_drafts();
                }
            }
            if let Some(c) = &self.col {
                self.drafts_fp = agent::drafts_fingerprint(&c.root);
            }
            self.poll_agents();
            self.info("MCP on: agent drafts and sessions are shown");
        } else {
            self.unload_drafts();
            self.follow = None;
            self.agent_sessions.clear();
            if self.share_view {
                self.share_view = false;
                self.shared = None;
                if let Some(c) = &self.col {
                    agent::share_view(&c.root, None);
                }
            }
            self.info("MCP off");
        }
    }

    /// Takes agent drafts out of the tree (the files stay).
    fn unload_drafts(&mut self) {
        let Some(col) = &mut self.col else { return };
        let ids: Vec<String> = col.drafts.iter().cloned().collect();
        for id in ids {
            col.replays.remove(&id);
            col.paths.remove(&id);
            self.edited.remove(&id);
        }
        col.drafts.clear();
        if self.selected.as_ref().is_some_and(|s| !col.replays.contains_key(s)) {
            self.selected = None;
            { self.end_pass(); self.pb = None; }
        }
    }

    /// The MCP slot in the status bar.
    fn mcp_status(&self, ui: &mut egui::Ui) {
        if self.col.is_none() {
            return;
        }
        if !self.mcp_on {
            ui.label(RichText::new("MCP off").small().weak()).on_hover_text("Agents' drafts and sessions are hidden. MCP > Enable MCP to show them.");
            return;
        }
        let n = self.agent_sessions.len();
        let mut text = match n {
            0 => "MCP: no sessions".to_string(),
            1 => "MCP: 1 session".to_string(),
            n => format!("MCP: {n} sessions"),
        };
        if let Some((_, s)) = &self.follow {
            text += &format!(" · following {s}");
        }
        if self.share_view {
            text += " · sharing view";
        }
        let servers: std::collections::BTreeSet<u64> = self.agent_sessions.iter().map(|s| s.pid).collect();
        let drafts = self.col.as_ref().map(|c| c.drafts.len()).unwrap_or(0);
        let hover = format!(
            "{} running dj mcp server{} with open sessions, {drafts} agent draft{} in the tree.{}",
            servers.len(),
            if servers.len() == 1 { "" } else { "s" },
            if drafts == 1 { "" } else { "s" },
            if self.share_view { " Agents can read what you are looking at (gui_view)." } else { "" }
        );
        ui.label(RichText::new(text).small().color(if n > 0 { DRAFT } else { Color32::GRAY })).on_hover_text(hover);
    }

    /// Picks up drafts that changed on disk, agent sessions, and what to share.
    fn poll_agents(&mut self) {
        if !self.mcp_on {
            return;
        }
        let Some(root) = self.col.as_ref().map(|c| c.root.clone()) else { return };
        self.agent_sessions = agent::sessions(&root);
        let sessions = &self.agent_sessions;
        self.taken_over.retain(|d| sessions.iter().any(|s| &s.draft == d));
        if self.show_drafts && self.hydrating.is_none() {
            let fp = agent::drafts_fingerprint(&root);
            if fp != self.drafts_fp {
                self.drafts_fp = fp;
                self.reload_drafts();
                // An agent's session may have recorded its draft: take the hashes from there.
                self.refresh_stored();
            }
        }
        if self.follow.is_some() {
            if !self.agent_sessions.iter().any(|s| Some(&(s.pid, s.session.clone())) == self.follow.as_ref()) {
                self.follow = None;
                self.info("The agent session you followed has closed.");
            } else {
                self.follow_agent();
            }
        }
        if self.share_view {
            let now = self.selected.clone().map(|id| (id, self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0)));
            if now != self.shared {
                if let (Some((id, f)), Some(col)) = (&now, &self.col) {
                    let title = col.replays.get(id).map(|r| r.title().to_string()).unwrap_or_default();
                    agent::share_view(&root, Some((id, *f, &title, col.is_draft(id))));
                }
                self.shared = now;
            }
        }
    }

    /// Picks up segment files that other tools changed, added or removed (an agent's
    /// `save_draft`, `dj clean`, `dj play --accept`, an editor). A segment with
    /// unsaved edits here keeps them, with a warning, since saving would overwrite the file.
    fn poll_segments(&mut self) {
        if self.hydrating.is_some() {
            return;
        }
        let Some(col) = &mut self.col else { return };
        let fp = agent::segments_fingerprint(&col.root);
        if fp == self.segments_fp {
            return;
        }
        let dir = col.root.join("segments");
        let mut on_disk: BTreeMap<String, (PathBuf, decomp_dj::format::Replay)> = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("segment") {
                    continue;
                }
                match decomp_dj::format::Replay::load(&p) {
                    Ok(r) => {
                        if let Ok(id) = r.id().map(String::from) {
                            on_disk.insert(id, (p, r));
                        }
                    }
                    // Possibly half written: look again at the next poll.
                    Err(_) => return,
                }
            }
        }
        self.segments_fp = fp;
        let mut changed: Vec<String> = vec![];
        let mut conflicts: Vec<String> = vec![];
        // New or changed files, parents first so a new child finds its parent.
        let mut pending: Vec<(String, (PathBuf, decomp_dj::format::Replay))> = on_disk.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        loop {
            let before = pending.len();
            pending.retain(|(id, (p, r))| {
                let same = col.replays.get(id).is_some_and(|m| m.to_text() == r.to_text());
                if same {
                    if col.is_draft(id) {
                        // A draft saved by an agent: now a segment.
                        col.drafts.remove(id);
                        col.paths.insert(id.clone(), p.clone());
                    }
                    return false;
                }
                if self.edited.contains(id) {
                    // Edited here (or deleted here and not saved yet): ours wins on save.
                    if col.replays.contains_key(id) {
                        conflicts.push(id.clone());
                    }
                    return false;
                }
                if r.parent().is_some_and(|par| !col.replays.contains_key(par)) {
                    return true; // wait for its parent
                }
                col.drafts.remove(id);
                col.replays.insert(id.clone(), r.clone());
                col.paths.insert(id.clone(), p.clone());
                changed.push(id.clone());
                false
            });
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        // Files gone: remove those segments, leaves first, unless edited here.
        loop {
            let gone: Vec<String> = col
                .replays
                .keys()
                .filter(|id| !col.is_draft(id) && !on_disk.contains_key(*id) && !self.edited.contains(*id))
                .filter(|id| !col.replays.values().any(|r| r.parent() == Some(id.as_str())))
                .cloned()
                .collect();
            if gone.is_empty() {
                break;
            }
            for id in gone {
                col.replays.remove(&id);
                col.paths.remove(&id);
                changed.push(id);
            }
        }
        if changed.is_empty() && conflicts.is_empty() {
            return;
        }
        self.note_saved(&changed);
        self.roll.invalidate();
        self.coverage_stale();
        if !conflicts.is_empty() {
            let names: Vec<String> = conflicts.iter().map(|id| format!("\"{}\"", col_title(self.col.as_ref(), id))).collect();
            self.error(format!("Changed on disk while you have unsaved edits: {}. Saving overwrites those files; File > Reload all from disk takes the disk's version.", names.join(", ")));
        } else {
            self.info(format!("Picked up {} segment file(s) changed on disk", changed.len()));
        }
        if let Some(sel) = self.selected.clone().filter(|s| changed.contains(s)) {
            if self.col.as_ref().is_some_and(|c| c.replays.contains_key(&sel)) {
                let frame = self.pb.as_ref().and_then(|p| p.shown());
                self.select(&sel);
                if let Some(f) = frame {
                    self.seek(f);
                }
            } else {
                self.selected = None;
                { self.end_pass(); self.pb = None; }
            }
        }
    }

    fn reload_drafts(&mut self) {
        let Some(col) = &mut self.col else { return };
        // Drafts edited here and not saved yet keep the edits.
        let keep: Vec<(String, decomp_dj::format::Replay)> = self.edited.iter().filter(|id| col.is_draft(id)).filter_map(|id| col.replays.get(id).map(|r| (id.clone(), r.clone()))).collect();
        let old: Option<decomp_dj::format::Replay> = self.selected.as_ref().and_then(|s| col.replays.get(s)).cloned();
        let deleted: Vec<String> = self.edited.iter().filter(|id| col.is_draft(id) && !col.replays.contains_key(*id)).cloned().collect();
        let Ok(changed) = col.load_drafts() else { return };
        for id in &deleted {
            // Deleted here, not saved yet: its file goes on save.
            col.replays.remove(id);
        }
        for id in &changed {
            if let Some(r) = col.replays.get(id) {
                self.saved_end.insert(id.clone(), ending_sig(r));
            }
        }
        for (id, r) in keep {
            col.replays.insert(id.clone(), r);
            col.drafts.insert(id);
        }
        let Some(sel) = self.selected.clone() else { return };
        if !changed.contains(&sel) || self.edited.contains(&sel) {
            return;
        }
        let Some(new) = col.replays.get(&sel).cloned() else {
            // The selected draft was saved or discarded.
            let next = old.as_ref().and_then(|o| o.parent()).map(String::from).filter(|p| col.replays.contains_key(p));
            { self.end_pass(); self.pb = None; }
            self.selected = None;
            if let Some(p) = next {
                self.select(&p);
            }
            return;
        };
        // Where the draft changed: the first frame whose lines differ, or the old end.
        let from = match &old {
            Some(o) => {
                let a: Vec<String> = o.lines.iter().map(|l| format!("{} {:?}", l.frame, l.command)).collect();
                let b: Vec<String> = new.lines.iter().map(|l| format!("{} {:?}", l.frame, l.command)).collect();
                let first = a.iter().zip(&b).position(|(x, y)| x != y).map(|i| new.lines[i].frame.min(o.lines[i].frame));
                let tail = if a.len() != b.len() { o.lines.get(b.len().min(a.len())).map(|l| l.frame).into_iter().chain(new.lines.get(a.len().min(b.len())).map(|l| l.frame)).min() } else { None };
                let ends = o.frames().unwrap_or(0).min(new.frames().unwrap_or(0));
                [first, tail, Some(ends)].into_iter().flatten().min().unwrap_or(0)
            }
            None => 0,
        };
        self.roll.invalidate();
        let frames = new.frames().unwrap_or(0);
        if let Some(pb) = &mut self.pb {
            if pb.id == sel {
                let shown = pb.shown();
                if let (Some(rec), Some(s)) = (pb.rec.take(), &mut self.session) {
                    let _ = s.keep_snapshots(rec);
                }
                pb.snapshots.retain(|k, _| *k <= from);
                pb.frames = frames;
                pb.end_hash = None;
                pb.checkpoint_miss = None;
                if self.follow.is_none() {
                    if let Some(c) = shown.filter(|c| *c >= from) {
                        self.seek(c.min(frames.saturating_sub(1)));
                    }
                }
            }
        }
    }

    /// Shows the followed session's draft at the agent's frame.
    fn follow_agent(&mut self) {
        let Some(f) = self.follow.clone() else { return };
        let Some(s) = self.agent_sessions.iter().find(|s| (s.pid, s.session.clone()) == f).cloned() else { return };
        if self.hydrating.is_some() {
            return;
        }
        let Some(col) = &mut self.col else { return };
        if !col.replays.contains_key(&s.draft) {
            let _ = col.load_drafts();
            self.drafts_fp = 0;
        }
        if !self.col.as_ref().is_some_and(|c| c.replays.contains_key(&s.draft)) {
            return;
        }
        if self.selected.as_deref() != Some(s.draft.as_str()) {
            self.select(&s.draft);
        }
        // The agent's core is at the start of `frame`: the picture is the frame before.
        let target = s.frame.saturating_sub(1);
        let shown = self.pb.as_ref().and_then(|p| p.shown());
        if shown != Some(target) && !self.playing && self.job.is_none() {
            self.seek(target);
            self.roll.follow_to(target);
        }
    }

    // ---- local database ----

    /// Records hashes the local database already knows (`hydrate::record_from_cache`): a
    /// dirty segment whose exact content and start were played before is clean again without
    /// playing. Unsaved changes, like any recorded hashes.
    fn hashes_from_cache(&mut self) {
        let skip: HashSet<String> = self.col.as_ref().map(|c| c.replays.keys().filter(|id| self.agent_owned(id)).cloned().collect()).unwrap_or_default();
        let (Some(col), Some(s)) = (&mut self.col, &self.session) else { return };
        let Some(c) = &s.cache else { return };
        let before: HashMap<String, decomp_dj::format::Replay> = col.replays.iter().filter(|(id, _)| col.tree_status(id) != Status::Clean).map(|(id, r)| (id.clone(), r.clone())).collect();
        match decomp_dj::hydrate::record_from_cache(col, c, &skip) {
            Ok(done) if !done.is_empty() => {
                let mut before = before;
                for (id, _) in &done {
                    if let Some(r) = before.remove(id) {
                        self.fold_into_undo(id, r);
                    }
                    self.edited.insert(id.clone());
                    self.dirty_from.remove(id);
                }
                self.info(format!("Recorded hashes for {} segment(s) from the local database, without playing. Save to keep them.", done.len()));
            }
            Ok(_) => {}
            Err(e) => self.error(format!("Reading hashes from the local database: {e:#}")),
        }
    }

    /// Which segments' current content is stored (the tree's "db" marks). Also records the
    /// hashes the database already knows.
    fn refresh_stored(&mut self) {
        self.hashes_from_cache();
        self.coverage_stale();
        self.stored_ids.clear();
        let (Some(col), Some(s)) = (&self.col, &self.session) else { return };
        let Some(c) = &s.cache else { return };
        if let Ok(keys) = decomp_dj::cache::current_keys(c.conn(), col, &c.setup) {
            for (id, (key, _)) in keys {
                if let Some(k) = key {
                    if c.content(&k).ok().flatten().is_some() {
                        self.stored_ids.insert(id);
                    }
                }
            }
        }
    }

    /// Clean all (`scope` None) or a subtree: plays the segments `how` asks for and records
    /// their hashes in memory (unsaved changes).
    fn start_verify(&mut self, scope: Option<String>, how: Clean) {
        let Some(col) = &self.col else { return };
        if self.session.as_ref().is_none_or(|s| s.cache.is_none()) {
            self.error("Cleaning needs the local database (Project > Use the local database).");
            return;
        }
        let what = match &scope {
            None => "all segments".to_string(),
            Some(id) => format!("\"{}\" and below", col_title(Some(col), id)),
        };
        // Drafts are unsaved segments and get cleaned too, unless an agent may still be writing one.
        let in_session: HashSet<String> = self.agent_sessions.iter().map(|s| s.draft.clone()).filter(|d| !self.taken_over.contains(d)).collect();
        let found = Hydrator::verify(col, scope.as_deref().map(|id| (id, true)), true, how == Clean::Everything).map(|mut h| {
            h.targets.retain(|id| !in_session.contains(id));
            if how == Clean::Unclean {
                h.targets.retain(|id| col.tree_status(id) != Status::Clean);
            }
            h
        });
        let stored = &self.stored_ids;
        let nothing = |h: &Hydrator| h.targets.iter().all(|id| col.tree_status(id) == Status::Clean && (how != Clean::Fill || stored.contains(id)));
        match found {
            Ok(h) if how != Clean::Everything && nothing(&h) => self.info(format!("Nothing to clean: {what} are clean and stored already.")),
            Ok(h) => {
                let before = h.targets.iter().filter_map(|id| col.replays.get(id).map(|r| (id.clone(), r.clone()))).collect();
                self.playing = false;
                self.job = None;
                { self.end_pass(); self.pb = None; }
                self.hydrating = Some(Hydrating { h, paused: false, what, before });
                self.base_next = None;
            }
            Err(e) => self.error(e),
        }
    }

    /// One slice of hydration; shows every 4th frame in the emulator view.
    fn hydrate_work(&mut self) {
        let Some(hy) = &mut self.hydrating else { return };
        if hy.paused {
            return;
        }
        let (Some(col), Some(s)) = (&mut self.col, &mut self.session) else { return };
        match hy.h.step(s, col, JOB_SLICE, 4) {
            Ok(events) => {
                if let Some(f) = hy.h.last_frame() {
                    let img = egui::ColorImage::from_rgb([f.width as usize, f.height as usize], &f.rgb);
                    match &mut self.texture {
                        Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                        None => self.texture = Some(egui_ctx().load_texture("preview", img, egui::TextureOptions::NEAREST)),
                    }
                }
                if !events.is_empty() {
                    // Verifying records hashes in memory: unsaved changes.
                    for e in events.iter().filter(|e| matches!(e.verdict, "recorded" | "cleaned" | "changed")) {
                        self.edited.insert(e.id.clone());
                        self.dirty_from.remove(&e.id);
                    }
                    self.refresh_stored();
                }
                if self.hydrating.as_ref().is_some_and(|h| h.h.done()) {
                    self.end_hydrate(false);
                }
            }
            Err(e) => {
                let mut hy = self.hydrating.take();
                if let Some(hy) = &mut hy {
                    self.undo_clean(hy);
                }
                let changed = hy.map_or(0, |h| h.h.events.iter().filter(|e| e.verdict == "changed").count());
                self.error(format!("Cleaning stopped: {e:#}"));
                self.reselect();
                if std::mem::take(&mut self.save_after_clean) {
                    self.after_clean(changed, Some(format!("{e:#}")));
                }
            }
        }
    }

    fn end_hydrate(&mut self, stopped: bool) {
        self.end_verify(stopped);
    }

    fn end_verify(&mut self, stopped: bool) {
        let Some(mut hy) = self.hydrating.take() else { return };
        self.undo_clean(&mut hy);
        let ev = &hy.h.events;
        let n = |v: &str| ev.iter().filter(|e| e.verdict == v).count();
        let (recorded, cleaned, changed, blocked) = (n("recorded"), n("cleaned"), n("changed"), n("blocked"));
        let played = ev.iter().filter(|e| e.action == "played").count();
        let mut msg = format!("{} {}: {played} played, {recorded} recorded, {cleaned} cleaned", if stopped { "Stopped cleaning" } else { "Cleaned" }, hy.what);
        if changed > 0 {
            msg += &format!(", {changed} with a changed ending (marked !)");
        }
        if recorded + cleaned + changed > 0 {
            msg += ". Unsaved: save to keep them, or undo";
        }
        if blocked > 0 {
            msg += &format!(". {blocked} couldn't be reached");
        }
        self.refresh_stored();
        self.info(msg);
        self.reselect();
        if std::mem::take(&mut self.save_after_clean) {
            self.after_clean(changed, stopped.then(|| "you stopped it".to_string()));
        }
    }

    /// Shows the selected segment again after hydration used the core.
    fn reselect(&mut self) {
        if let Some(id) = self.selected.clone() {
            if self.col.as_ref().is_some_and(|c| c.replays.contains_key(&id)) {
                self.select(&id);
            }
        }
    }

    fn set_database(&mut self, on: bool) {
        let Some(col) = &self.col else { return };
        let mut local = col.load_local().unwrap_or_default();
        // Explicit either way, so it also wins over a `database` in config.toml.
        local.database = Some(on);
        if let Err(e) = col.save_local(&local) {
            self.error(e);
            return;
        }
        { self.end_pass(); self.pb = None; }
        self.open_session();
        self.refresh_stored();
        self.reselect();
        self.info(if on { "Using the local database" } else { "Running without the local database" });
    }

    fn set_max_memory(&mut self, mb: u64) {
        self.max_memory_mb = mb;
        if let Some(col) = &self.col {
            let mut local = col.load_local().unwrap_or_default();
            local.max_memory_mb = Some(mb);
            let _ = col.save_local(&local);
        }
        if let Some(pb) = &mut self.pb {
            let at = pb.ran;
            trim_snapshots(&mut pb.snapshots, at, mb * 1024 * 1024);
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let has = self.pb.is_some();
            let idle = self.hydrating.is_none();
            let (parent, kids): (Option<String>, Vec<(String, String)>) = match (&self.col, &self.selected) {
                (Some(col), Some(id)) if col.replays.contains_key(id) => (
                    col.replays[id].parent().map(String::from),
                    col.children(Some(id)).into_iter().map(|k| (k.to_string(), col_title(Some(col), k))).collect(),
                ),
                _ => (None, vec![]),
            };
            // Marker frames in this segment, before and after the frame shown.
            let (prev_mk, next_mk) = match (&self.pb, &self.col) {
                (Some(pb), Some(col)) => {
                    let at = pb.shown().unwrap_or(0);
                    let frames: Vec<u64> = col.replays.get(&pb.id).map(|r| r.lines.iter().filter(|l| is_marker(&l.command)).map(|l| l.frame).collect()).unwrap_or_default();
                    (frames.iter().filter(|f| **f < at).max().copied(), frames.iter().filter(|f| **f > at && **f < pb.frames).min().copied())
                }
                _ => (None, None),
            };
            let icon = egui::vec2(30.0, ui.spacing().interact_size.y);

            let at = self.pb.as_ref().and_then(|pb| pb.shown());
            let at_start = at.is_some_and(|f| f == 0);
            let at_end = self.pb.as_ref().is_some_and(|pb| at.is_some_and(|f| f + 1 >= pb.frames));

            // Start: to frame 0, or from frame 0 to the end of the parent.
            let start_ok = has && idle && (!at_start || parent.is_some());
            let tip = match (&parent, at_start) {
                (Some(p), true) => format!("Back to the end of the parent segment, \"{}\" (Home goes to frame 0)", col_title(self.col.as_ref(), p)),
                (_, false) => "Go to start (Home). At the start, this goes back to the end of the parent segment".to_string(),
                (None, true) => String::new(),
            };
            if ui.add_enabled(start_ok, egui::Button::new("⏮").min_size(icon)).on_hover_text(tip).on_disabled_hover_text("At the start of a root segment").clicked() {
                if at_start {
                    if let Some(p) = parent.clone() {
                        self.segment_back(&p);
                    }
                } else {
                    self.playing = false;
                    self.seek(0);
                    self.roll.follow_to(0);
                }
            }
            let r = ui.add_enabled(has && prev_mk.is_some(), egui::Button::new("").min_size(icon)).on_hover_text("Previous marker in this segment").on_disabled_hover_text("No marker before the playhead in this segment");
            paint_skip(ui, &r, false, true);
            if r.clicked() {
                if let Some(f) = prev_mk {
                    self.playing = false;
                    self.seek(f);
                    self.roll.follow_to(f);
                }
            }

            ui.add_enabled_ui(has, |ui| {
                if ui.button("⏴").on_hover_text("Step back (left arrow)").clicked() {
                    self.step_by(-1);
                }
                if ui.add(egui::Button::new(if self.playing { "⏸" } else { "▶" }).min_size(egui::vec2(34.0, icon.y))).on_hover_text("Play / pause (Space)").clicked() {
                    self.toggle_play();
                }
                if ui.button("⏵").on_hover_text("Step forward (right arrow)").clicked() {
                    self.step_by(1);
                }
            });

            // Next marker; end (split button), continue.
            let r = ui.add_enabled(has && next_mk.is_some(), egui::Button::new("").min_size(icon)).on_hover_text("Next marker in this segment").on_disabled_hover_text("No marker after the playhead in this segment");
            paint_skip(ui, &r, true, true);
            if r.clicked() {
                if let Some(f) = next_mk {
                    self.playing = false;
                    self.seek(f);
                    self.roll.follow_to(f);
                }
            }
            let sticky = self.selected.as_deref().and_then(|id| self.sticky_child(id));
            // At the end, the main part goes to the sticky segment, else the only child, else opens the list.
            let child_target = sticky.clone().or_else(|| (kids.len() == 1).then(|| kids[0].0.clone()));
            let title_of = |k: &str| kids.iter().find(|(c, _)| c == k).map(|(_, t)| t.clone()).unwrap_or_default();
            let mut pick = None;
            let mut to_end = false;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let cr = ui.visuals().widgets.inactive.corner_radius;
                let left = egui::CornerRadius { nw: cr.nw, sw: cr.sw, ne: 0, se: 0 };
                let right = egui::CornerRadius { nw: 0, sw: 0, ne: cr.ne, se: cr.se };
                let main = egui::Button::new("⏭").min_size(icon).corner_radius(left);
                let main_ok = has && idle && (!at_end || !kids.is_empty());
                if at_end && main_ok && child_target.is_none() {
                    // At the end with several children and none to follow: the main part opens the list.
                    let resp = egui::containers::menu::MenuButton::from_button(main).ui(ui, |ui| child_list(ui, &kids, None, &mut pick)).0;
                    resp.on_hover_text(format!("Forward into a child segment ({} children)", kids.len()));
                } else {
                    let tip = match (&child_target, at_end) {
                        (_, false) => "Go to end (End). At the end, this goes forward into the child segment".to_string(),
                        (Some(c), true) if sticky.is_some() => format!("Forward into \"{}\", where you were before", title_of(c)),
                        (Some(c), true) => format!("Forward into the child segment, \"{}\"", title_of(c)),
                        (None, true) => String::new(),
                    };
                    let r = ui.add_enabled(main_ok, main).on_hover_text(tip).on_disabled_hover_text("At the end of a segment with no children");
                    if r.clicked() {
                        if at_end {
                            pick = child_target.clone();
                        } else {
                            to_end = true;
                        }
                    }
                }
                // The side arrow: always there, so nothing moves; faint with a single child.
                let arrow = egui::Button::new("").min_size(egui::vec2(16.0, icon.y)).corner_radius(right);
                ui.add_space(1.0);
                ui.add_enabled_ui(idle && !kids.is_empty(), |ui| {
                    let r = egui::containers::menu::MenuButton::from_button(arrow).ui(ui, |ui| child_list(ui, &kids, sticky.as_ref(), &mut pick)).0;
                    let r = r.on_hover_text("Pick a child segment").on_disabled_hover_text("This segment has no children");
                    let c = icon_color(ui, &r);
                    let c = if kids.len() > 1 { c } else { c.gamma_multiply(0.4) };
                    let m = r.rect.center();
                    ui.painter().add(egui::Shape::convex_polygon(vec![egui::pos2(m.x - 3.5, m.y - 2.0), egui::pos2(m.x + 3.5, m.y - 2.0), egui::pos2(m.x, m.y + 2.5)], c, egui::Stroke::NONE));
                });
            });
            if to_end {
                self.playing = false;
                let e = self.pb.as_ref().map_or(0, |p| p.frames.saturating_sub(1));
                self.seek(e);
                self.roll.follow_to(e);
            }
            if let Some(k) = pick {
                self.segment_forward(&k);
            }
            // What happens at the end of a segment.
            ui.allocate_ui_with_layout(egui::vec2(64.0, icon.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let cur = self.end_mode;
                egui::ComboBox::from_id_salt("end_mode")
                    .selected_text(cur.label())
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        for m in EndMode::ALL {
                            ui.selectable_value(&mut self.end_mode, m, m.label()).on_hover_text(m.tip());
                        }
                        ui.separator();
                        ui.checkbox(&mut self.stop_on_changed, "Stop at a changed ending")
                            .on_hover_text("Pause as soon as a segment records an ending that differs from its saved file (the !), so you can undo before the segments below it replay. Off: playback carries on, since a child always starts from where its parent really ends.");
                    })
                    .response
                    .on_hover_text("Behavior at the end of the segment");
            });
            ui.separator();
            self.record_button(ui, icon);
            ui.separator();

            // Speed.
            ui.add_enabled_ui(has, |ui| {
                let mut new = None;
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.add_enabled(self.speed > 0, egui::Button::new("−")).on_hover_text("Slower").clicked() {
                    new = Some(self.speed - 1);
                }
                ui.allocate_ui_with_layout(egui::vec2(76.0, icon.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("speed")
                        .selected_text(SPEEDS[self.speed].1)
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            for (i, (_, label)) in SPEEDS.iter().enumerate() {
                                if ui.selectable_label(self.speed == i, *label).clicked() {
                                    new = Some(i);
                                }
                            }
                        })
                        .response
                        .on_hover_text("Playback speed");
                });
                if ui.add_enabled(self.speed + 1 < SPEEDS.len(), egui::Button::new("+")).on_hover_text("Faster").clicked() {
                    new = Some(self.speed + 1);
                }
                if let Some(i) = new {
                    self.speed = i;
                    self.clock = 0.0;
                }
            });
        });
        ui.horizontal(|ui| {
            // Fixed height: the row is empty while a load runs, and the preview must not move.
            ui.set_min_height(20.0);
            if let (Some(pb), Some(col)) = (&self.pb, &self.col) {
                let shown = pb.shown().unwrap_or(0);
                ui.monospace(format!("frame {shown:>5} / {}", pb.frames.saturating_sub(1)));
                ui.label(RichText::new(format!("power-on frame {}", pb.start_frame + shown)).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The segment's status, as the tree shows it.
                    let status = col.tree_status(&pb.id);
                    let r = &col.replays[&pb.id];
                    let parent_clean = r.parent().is_none_or(|p| col.tree_status(p) == Status::Clean);
                    let (text, color) = if self.ending_changed(&pb.id) {
                        ("! ending changed since the last save".to_string(), CHANGED)
                    } else if status == Status::Clean {
                        ("● clean".to_string(), status_color(status))
                    } else if let Some(m) = pb.checkpoint_miss {
                        (format!("● checkpoint {m} differs: play to the end to record the new ending"), status_color(status))
                    } else if !parent_clean {
                        ("● dirty: waiting on a segment above".to_string(), status_color(status))
                    } else if r.get("end-hash").is_none() {
                        ("● dirty: play to the end to record its hashes".to_string(), status_color(status))
                    } else {
                        ("● dirty: play to the end to re-check".to_string(), status_color(status))
                    };
                    ui.label(RichText::new(text).color(color));
                });
            }
        });
    }

    /// The Compare menu: the comparison core, Run, Sync now, Check parity and the parity
    /// actions.
    fn compare_bar_menu(&mut self, ui: &mut egui::Ui) {
        self.compare_menu(ui);
        ui.separator();
        let loaded = self.compare.is_some();
        let no_core = "Load a comparison core first";
        let was = self.compare_run;
        ui.add_enabled(loaded, egui::Checkbox::new(&mut self.compare_run, "Run"))
            .keep_open()
            .on_hover_text("Run comparison: off pauses the comparison core without unloading it; on syncs it again")
            .on_disabled_hover_text(no_core);
        if self.compare_run && !was {
            self.checker.lose_step();
            self.compare_need_sync = true;
        }
        let busy = self.playing || self.job.is_some() || self.hydrating.is_some();
        let sync_why = if !loaded {
            no_core
        } else if !self.compare_run {
            "Run is off"
        } else {
            "Busy: pause playback and wait for the clean or seek to finish"
        };
        if ui
            .add_enabled(loaded && self.compare_run && !busy, egui::Button::new("Sync now"))
            .on_hover_text("Bring the comparison core to the baseline's state at the frame shown")
            .on_disabled_hover_text(sync_why)
            .clicked()
        {
            // As playing on after a failure does: drop the failure and sync first.
            self.compare_failed = false;
            self.compare_problem = None;
            self.checker.lose_step();
            self.compare_need_sync = true;
        }
        let how = if self.compare.as_ref().is_some_and(|c| c.has_check()) { "with the core's dj_check_parity" } else { "by comparing system RAM (the core has no dj_check_parity)" };
        ui.add_enabled(loaded, egui::Checkbox::new(&mut self.check_parity, "Check parity"))
            .keep_open()
            .on_hover_text(format!("After every frame both cores run, check that they agree, {how}. A failure stops playback at that frame."))
            .on_disabled_hover_text(no_core);
        ui.separator();
        if self.rerun.is_some() {
            let left = self.rerun.as_ref().map_or(0, |r| r.queue.len() + r.current.is_some() as usize);
            if ui.button(format!("Stop re-running ({left} to go)")).clicked() {
                self.rerun_stop();
            }
        } else {
            let old: Vec<i64> = self.parity.as_ref().map(|p| p.failures.iter().filter(|f| p.is_old(f)).map(|f| f.id).collect()).unwrap_or_default();
            let why = if !loaded { no_core } else { "No failures from an older build" };
            if ui
                .add_enabled(!old.is_empty(), egui::Button::new(format!("Re-run old failures ({})", old.len())))
                .on_hover_text("Re-run every failure from an older build: those that pass now are cleared")
                .on_disabled_hover_text(why)
                .clicked()
            {
                self.rerun_start(old);
            }
        }
    }

    /// Compare > Comparison core: picks the comparison core, or None.
    fn compare_menu(&mut self, ui: &mut egui::Ui) {
        let names: Vec<String> = self.col.as_ref().map(|c| c.config.compare.iter().map(|e| e.name.clone()).collect()).unwrap_or_default();
        let busy = self.hydrating.is_some() || self.job.is_some();
        let cur = self.compare.as_ref().map(|c| c.name.clone());
        let mut pick = None;
        ui.add_enabled_ui(!names.is_empty() && !busy, |ui| {
            ui.menu_button(format!("Comparison core: {}", cur.as_deref().unwrap_or("none")), |ui| {
                if ui.radio(cur.is_none(), "None").clicked() && cur.is_some() {
                    pick = Some(None);
                }
                for n in &names {
                    if ui.radio(cur.as_ref() == Some(n), n).clicked() && cur.as_ref() != Some(n) {
                        pick = Some(Some(n.clone()));
                    }
                }
            })
            .response
            .on_disabled_hover_text(if names.is_empty() { "The project has no [[compare]] entries" } else { "Busy: wait for the clean or seek to finish" });
        });
        // A comparison core that crashed or hung is loaded again, in a new worker.
        if let Some(why) = self.compare.as_ref().and_then(|c| c.dead()).map(String::from) {
            if ui.button("Restart comparison core").on_hover_text(format!("It stopped: {why}")).clicked() {
                pick = Some(cur.clone());
            }
        }
        if let Some(p) = pick {
            self.select_compare(p);
        }
    }

    /// Selects the parent and goes to its last frame.
    fn segment_back(&mut self, parent: &str) {
        self.playing = false;
        self.route.clear();
        self.route_target = None;
        self.select(parent);
        let e = self.col.as_ref().and_then(|c| c.replays.get(parent)).and_then(|r| r.frames().ok()).unwrap_or(1).saturating_sub(1);
        self.seek(e);
        self.roll.follow_to(e);
    }

    /// Goes to frame 0 of `child`: straight on from the current state when the playhead is at
    /// the end of this segment (as continue does), otherwise by loading the child.
    fn segment_forward(&mut self, child: &str) {
        self.route.clear();
        self.route_target = None;
        let at_end = self.pb.as_ref().is_some_and(|pb| pb.ran == pb.frames && pb.end_hash.is_some());
        let unsaved = self.edited.contains(child) || self.selected.as_ref().is_some_and(|s| self.edited.contains(s));
        if at_end && !unsaved && self.job.is_none() {
            if let Err(e) = self.continue_into(child) {
                self.playing = false;
                self.error(e);
                return;
            }
            // Paused (a step or ⏭): run and show the child's frame 0, so the playhead has a
            // frame to stand on. Playing on, the next tick runs it.
            if !self.playing {
                self.seek(0);
            }
            return;
        }
        self.playing = false;
        self.select(child);
        self.roll.follow_to(0);
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        // The picture fills what's left above the transport (two rows below it).
        let full = ui.available_rect_before_wrap();
        let bar_h = 2.0 * ui.spacing().interact_size.y + 3.0 * ui.spacing().item_spacing.y + 8.0;
        let avail = egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, (full.max.y - bar_h).max(full.min.y)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(avail), |ui| self.picture(ui, avail));
        let bar = egui::Rect::from_min_max(egui::pos2(full.min.x, avail.max.y + 4.0), full.max);
        if self.recording {
            // Unmistakable while on: a red frame around the picture and a tinted transport.
            ui.painter().rect_stroke(avail.shrink(1.0), 0.0, egui::Stroke::new(2.0, REC_RED), egui::StrokeKind::Inside);
            ui.painter().rect_filled(bar.expand2(egui::vec2(0.0, 2.0)), 4.0, REC_RED.gamma_multiply(0.18));
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| self.transport(ui));
        ui.allocate_rect(full, egui::Sense::hover());
    }

    fn picture(&mut self, ui: &mut egui::Ui, avail: egui::Rect) {
        ui.painter().rect_filled(avail, 0.0, Color32::from_gray(12));
        if let Some(err) = &self.core_error {
            ui.scope_builder(egui::UiBuilder::new().max_rect(avail.shrink(24.0)), |ui| {
                ui.label(RichText::new(err).color(Color32::from_rgb(240, 160, 60)));
            });
            return;
        }
        if self.compare.is_none() {
            self.baseline_picture(ui, avail);
            return;
        }
        // Side by side, each under a label, with a divider to drag.
        let head = 20.0;
        let gap = 8.0;
        let x = avail.min.x + avail.width() * self.compare_split;
        let left = egui::Rect::from_min_max(avail.min, egui::pos2(x - gap / 2.0, avail.max.y));
        let right = egui::Rect::from_min_max(egui::pos2(x + gap / 2.0, avail.min.y), avail.max);
        let div = egui::Rect::from_min_max(egui::pos2(x - gap / 2.0, avail.min.y), egui::pos2(x + gap / 2.0, avail.max.y));
        let r = ui.interact(div, egui::Id::unique("compare_divider"), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if r.dragged() {
            if let Some(p) = r.interact_pointer_pos() {
                self.compare_split = ((p.x - avail.min.x) / avail.width().max(1.0)).clamp(0.15, 0.85);
            }
        }
        if r.double_clicked() {
            self.compare_split = 0.5;
        }
        let line = if r.hovered() || r.dragged() { Color32::from_gray(140) } else { Color32::from_gray(60) };
        ui.painter().vline(x, avail.y_range(), egui::Stroke::new(1.0, line));
        let split = |rc: egui::Rect| (egui::Rect::from_min_max(rc.min, egui::pos2(rc.max.x, rc.min.y + head)), egui::Rect::from_min_max(egui::pos2(rc.min.x, rc.min.y + head), rc.max));
        let (lh, lb) = split(left);
        let (rh, rb) = split(right);
        let base_name = self.col.as_ref().map(|c| c.config.core.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "core".into());
        ui.scope_builder(egui::UiBuilder::new().max_rect(lh.shrink2(egui::vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.label(RichText::new("Baseline").strong());
            ui.label(RichText::new(base_name).weak());
        });
        self.baseline_picture(ui, lb);

        // The comparison core's state, and what's shown over its picture.
        let name = self.compare.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        let (status, color, over) = if !self.compare_run {
            ("paused".to_string(), Color32::from_gray(150), Some("Run comparison is off".to_string()))
        } else if let (true, Some(p)) = (self.compare_failed, &self.compare_problem) {
            ("stopped".to_string(), Color32::from_rgb(240, 100, 90), Some(p.clone()))
        } else if let Some(p) = &self.compare_problem {
            ("out of sync".to_string(), Color32::from_rgb(240, 160, 60), Some(p.clone()))
        } else if self.checker.at.is_some() {
            ("in step".to_string(), Color32::from_rgb(120, 200, 120), None)
        } else {
            ("waiting to sync".to_string(), Color32::from_gray(150), Some("Syncs at the next frame shown".to_string()))
        };
        let mut sync_now = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rh.shrink2(egui::vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.label(RichText::new(&name).strong());
            ui.label(RichText::new(&status).color(color));
            if self.compare_run && self.checker.at.is_some() && self.check_parity {
                ui.label(RichText::new("checking parity").weak());
            }
            if self.compare_run && self.checker.at.is_none() && !self.compare_failed {
                let busy = self.playing || self.job.is_some() || self.hydrating.is_some();
                sync_now = ui.add_enabled(!busy, egui::Button::new("Sync now").small()).on_hover_text("Bring the comparison core to the baseline's state at the frame shown").clicked();
            }
        });
        if sync_now {
            self.compare_problem = None;
            self.compare_need_sync = true;
        }
        let aspect = self.compare.as_ref().map_or(0.0, |c| c.aspect());
        let rect = self.compare_texture.as_ref().map(|t| fit_picture(ui, t, aspect, rb));
        if let Some(text) = over {
            let rc = rect.unwrap_or(rb);
            if rect.is_some() {
                ui.painter().rect_filled(rc, 0.0, Color32::from_black_alpha(160));
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(rc.shrink(16.0)).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
                ui.add_space((rc.height() / 2.0 - 40.0).max(0.0));
                ui.label(RichText::new(text).color(Color32::from_gray(225)));
            });
        }
    }

    fn baseline_picture(&self, ui: &mut egui::Ui, avail: egui::Rect) {
        let Some(tex) = &self.texture else { return };
        let rect = fit_picture(ui, tex, self.aspect, avail);
        // A frame from before the latest edit, while the new one is worked out.
        if self.pb.as_ref().is_some_and(|p| p.picture_stale) {
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(150));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "recomputing after the edit…", egui::FontId::proportional(14.0), Color32::from_gray(220));
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some((m, err)) = &self.message {
                let t = RichText::new(m).small();
                ui.label(if *err { t.color(Color32::from_rgb(230, 90, 80)) } else { t.weak() });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Fixed slot for the progress bar, so nothing moves when it appears.
                let slot = egui::vec2(260.0, 16.0);
                if let Some(hy) = &self.hydrating {
                    let n = hy.h.targets.len().max(1);
                    let (frac, name) = match hy.h.progress() {
                        Some((id, f, frames)) => ((hy.h.i as f32 + f as f32 / frames.max(1) as f32) / n as f32, col_title(self.col.as_ref(), id)),
                        None => (hy.h.i as f32 / n as f32, String::new()),
                    };
                    let text = format!("{} {}/{}{}", if hy.paused { "Paused" } else if hy.h.verify { "Cleaning" } else { "Hydrating" }, (hy.h.i + 1).min(n), n, if name.is_empty() { String::new() } else { format!(": \"{name}\"") });
                    ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(slot.x).desired_height(slot.y).text(RichText::new(text).small()).corner_radius(3.0));
                    let mut stop = false;
                    let mut toggle = false;
                    if ui.small_button("Stop").on_hover_text(if hy.h.verify { "Stop cleaning; hashes recorded so far stay (unsaved)" } else { "Stop hydrating; finished segments stay stored" }).clicked() {
                        stop = true;
                    }
                    if ui.small_button(if hy.paused { "Resume" } else { "Pause" }).clicked() {
                        toggle = true;
                    }
                    if toggle {
                        if let Some(h) = &mut self.hydrating {
                            h.paused = !h.paused;
                        }
                    }
                    if stop {
                        self.end_hydrate(true);
                    }
                    return;
                }
                match self.job_progress() {
                    Some((f, text)) => {
                        ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).desired_width(slot.x).desired_height(slot.y).text(RichText::new(text).small()).corner_radius(3.0));
                    }
                    None => {
                        ui.allocate_space(slot);
                    }
                }
                if let Some(col) = &self.col {
                    ui.label(RichText::new(format!("{} {}", col.config.core.name, col.config.core.build)).small().weak());
                    let n = self.unsaved().len();
                    if n > 0 {
                        ui.label(RichText::new(format!("{n} unsaved")).small().color(Color32::from_rgb(240, 210, 90)));
                    }
                }
                ui.separator();
                self.mcp_status(ui);
            });
        });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, Modifiers};
        // While recording, the game's keys win: only Escape (stop recording), Space as
        // play/pause in real time, and Ctrl/Cmd shortcuts stay.
        let plain = !self.recording;
        if self.recording && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.set_recording(false);
        }
        let rec_space = self.recording && !self.rec_cfg.frame_by_frame && !self.bindings.is_button(Key::Space) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space));
        let (space, right, left, home, end, save) = ctx.input_mut(|i| {
            (
                rec_space || (plain && i.consume_key(Modifiers::NONE, Key::Space)),
                plain && i.consume_key(Modifiers::NONE, Key::ArrowRight),
                plain && i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                plain && i.consume_key(Modifiers::NONE, Key::Home),
                plain && i.consume_key(Modifiers::NONE, Key::End),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        let add_event = plain && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::E));
        // Redo first: Cmd+Z alone would also match Cmd+Shift+Z.
        let (undo, redo, mut cut, mut copy, mut paste) = ctx.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
            let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
            (undo, redo, i.consume_key(Modifiers::COMMAND, Key::X), i.consume_key(Modifiers::COMMAND, Key::C), i.consume_key(Modifiers::COMMAND, Key::V))
        });
        // Some platforms deliver the clipboard keys as events instead of key presses.
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Cut => {
                    cut = true;
                    false
                }
                egui::Event::Copy => {
                    copy = true;
                    false
                }
                egui::Event::Paste(_) => {
                    paste = true;
                    false
                }
                _ => true,
            })
        });
        if redo {
            self.redo();
        } else if undo {
            self.undo();
        }
        if cut {
            self.cut();
        } else if copy {
            self.copy();
        }
        if paste {
            self.paste();
        }
        // Cmd+A selects every press in the piano roll. Bare A may be a recording key, but
        // Cmd+A never is; a focused text field keeps Cmd+A (shortcuts() returns early then).
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) && self.selected.is_some() {
            self.roll.select_all();
        }
        let rename = plain && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::F2));
        if rename && self.node_dialog.is_none() && self.frame_dialog.is_none() && self.renaming.is_none() {
            if let (Some(id), Some(col)) = (&self.selected, &self.col) {
                let title = col.replays[id].title().to_string();
                self.renaming = Some((id.clone(), title, true));
                // Make sure the row is visible.
                if self.hoist.as_ref().is_some_and(|h| col.chain(id).is_ok_and(|c| !c.contains(&h.as_str()))) {
                    self.hoist = None;
                }
            }
        }
        if add_event && self.pb.is_some() {
            let f = self.pb.as_ref().and_then(|p| p.shown()).unwrap_or(0);
            self.open_frame_dialog(f, "[EVENT] ");
        }
        if space {
            self.toggle_play();
        }
        if right {
            self.step_by(1);
        }
        if left {
            self.step_by(-1);
        }
        if home {
            self.playing = false;
            self.seek(0);
            self.roll.follow_to(0);
        }
        if end {
            self.playing = false;
            let e = self.pb.as_ref().map_or(0, |p| p.frames.saturating_sub(1));
            self.seek(e);
            self.roll.follow_to(e);
        }
        if save {
            self.save();
        }
    }
}

thread_local! {
    static CTX: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
}

/// Draws `tex` as large as fits in `avail` at display aspect `aspect` (0: square pixels),
/// in whole steps once it's twice its size. Returns where it went.
fn fit_picture(ui: &egui::Ui, tex: &egui::TextureHandle, aspect: f32, avail: egui::Rect) -> egui::Rect {
    let [w, h] = tex.size();
    let disp_w = if aspect > 0.1 { h as f32 * aspect } else { w as f32 };
    let size = egui::vec2(disp_w, h as f32);
    let mut scale = (avail.width() / size.x).min(avail.height() / size.y);
    if scale >= 2.0 {
        scale = scale.floor();
    }
    let rect = egui::Rect::from_center_size(avail.center(), size * scale);
    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    rect
}

fn egui_ctx() -> egui::Context {
    CTX.with(|c| c.borrow().clone().expect("egui context"))
}

/// A normal close: nothing for the next launch to recover.
impl Drop for App {
    fn drop(&mut self) {
        if let Some(col) = &self.col {
            decomp_dj::recovery::clear(&col.root);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        // Closing the window (its close button, Cmd+Q) with unsaved edits asks first.
        if ctx.input(|i| i.viewport().close_requested()) {
            self.parity_save(true);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok && self.unsaved_edits() > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.discard_dialog = Some(Discard::Quit);
        }
        self.shortcuts(&ctx);
        if let Some(t) = self.ctx_copy.take() {
            ctx.copy_text(t);
        }
        let dt = ctx.input(|i| i.unstable_dt).min(0.1) as f64;
        let now = ctx.input(|i| i.time);
        decomp_dj::crash::heartbeat();
        if now - self.recovery_checked > 1.0 {
            self.recovery_checked = now;
            self.write_recovery();
        }
        if now - self.agent_polled > 0.5 {
            self.agent_polled = now;
            if let Some(m) = &mut self.gui_mark {
                m.touch();
            }
            self.poll_segments();
            self.poll_agents();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(600));
        // Lines logged by other threads (workers, cores) show without waiting for input.
        if self.show_console {
            if decomp_dj::console::count() != self.console_seen {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        self.work();
        self.coverage_update(ctx.input(|i| i.time));
        self.update_sticky_tip();
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
            if self.show_console {
                // The list fills the panel itself; asking for the space left on top of that would
                // make the panel a little taller every frame.
                egui::Panel::bottom("console").resizable(true).default_size(160.0).size_range(60.0..=700.0).show(ui, |ui| self.console_panel(ui));
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
                    prev: col.replays.get(id).and_then(|r| r.parent()).map(String::from),
                    next: self.next_neighbour(),
                    dirty: self.dirty_band(id),
                };
                action = self.roll.show(ui, col, id, shown, self.playing, &nb);
            } else {
                ui.label("Select a segment to see its inputs.");
            }
        });
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.dialogs(&ctx);
        if let Some(d) = &mut self.mapping {
            match d.show(&ctx) {
                Some(record::MappingResult::Save(b)) => {
                    self.bindings = b;
                    self.mapping = None;
                    if let Some(col) = &self.col {
                        let mut local = col.load_local().unwrap_or_default();
                        local.keyboard = Some(self.bindings.to_local());
                        if let Err(e) = col.save_local(&local) {
                            self.error(e);
                        }
                    }
                }
                Some(record::MappingResult::Cancel) => self.mapping = None,
                None => {}
            }
        }

        match action {
            Some(roll::Action::Seek(f)) => {
                self.playing = false;
                self.seek(f);
            }
            Some(roll::Action::Edit { edits, from }) => self.apply_edit("Edit notes", edits, from),
            Some(roll::Action::OpenFrame(f)) => self.open_frame_dialog(f, ""),
            None => {}
        }
    }
}
