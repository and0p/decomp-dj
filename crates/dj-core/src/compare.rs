//! Comparison cores: a second libretro core that plays beside the baseline, driven by the
//! same input lines (docs/config.md, docs/connectors.md#comparison-cores).
//!
//! There are two kinds. A core that takes events (`dj_handle_event`) is told where the
//! baseline has got to, with a handle to read it through, and runs itself and compares
//! inside those calls. The same emulator as the baseline is brought to its state by
//! loading its save state, runs one frame after it, and is judged by its system RAM.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use crate::collection::{sha256_file, Collection};
use crate::format::{Command, InputKey};
use crate::libretro::{Core, Frame, COMPARISON, MEMORY_SYSTEM_RAM};
use crate::player::{restore_inputs_on, write_memory};

/// How a comparison core follows the baseline.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncMode {
    /// The core takes events: it must have `dj_handle_event`, and copies what it needs from
    /// the baseline itself.
    #[default]
    #[serde(rename = "events")]
    Events,
    /// Load the baseline's save state: only for the same emulator (or one with the same
    /// state format), to compare builds or options.
    #[serde(rename = "state")]
    State,
}

/// What a comparison core loads: by default the project's ROM.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Content {
    /// `content = "path"`: a file of its own (relative paths are from the project folder).
    Path(PathBuf),
    /// `content = false`: nothing; `content = true`: the project's ROM.
    Rom(bool),
}

/// `[[compare]]` in config.toml (or local.toml, merged by `name`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CompareConfig {
    pub name: String,
    /// The core library on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// A free-form id of the build, for people.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub build: String,
    /// Pins the core library, like `[core] sha256`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    #[serde(default)]
    pub sync: SyncMode,
    /// Frames to play before a place jumped to for a parity check, so that a core which
    /// can't copy the baseline between frames has copied it by then (docs/config.md).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub preroll: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
}

type DiffsFn = unsafe extern "C" fn(*mut DjParityDiff, usize) -> usize;

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// `struct dj_parity_diff` (docs/connectors.md#parity): one difference, in the original's terms.
#[repr(C)]
#[derive(Clone, Copy)]
struct DjParityDiff {
    label: *const c_char,
    region: *const c_char,
    address: u64,
    symbol: *const c_char,
    expected: *const u8,
    actual: *const u8,
    len: usize,
}

/// The most differences kept for one failure; the rest are counted.
pub const DIFF_CAP: usize = 100;
/// The most bytes kept of each side of a difference.
pub const DIFF_BYTES: usize = 64;

/// One thing that differed on a failing frame (docs/connectors.md#parity).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Diff {
    /// What the value means: "Player 2 x coordinate".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The original's memory region: "RAM", "SRAM". None: the value has no one place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Offset in `region`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<u64>,
    /// A data symbol in the original's symbol map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// The value's size in bytes (the bytes below are cut to [`DIFF_BYTES`]).
    pub len: u64,
    /// The original's (the baseline's) bytes.
    pub expected: Vec<u8>,
    /// The comparison core's bytes.
    pub actual: Vec<u8>,
}

/// What differed on a failing frame: up to [`DIFF_CAP`] differences, and how many in all.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Diffs {
    pub list: Vec<Diff>,
    pub total: u64,
}

impl Diff {
    /// Where it is: "RAM 0x0002", a symbol, or None for a value with no one place.
    pub fn place(&self) -> Option<String> {
        match (&self.symbol, &self.region, self.address) {
            // A symbol names the place for a person; the address beside it is what dj traces.
            (Some(s), Some(r), Some(a)) => Some(format!("{s}, {r} 0x{a:04X}")),
            (Some(s), _, _) => Some(s.clone()),
            (None, Some(r), Some(a)) => Some(format!("{r} 0x{a:04X}")),
            (None, Some(r), None) => Some(r.clone()),
            _ => None,
        }
    }

    /// One line: `mixed state (RAM 0x0002): 47 here, 12 in the original`.
    pub fn describe(&self) -> String {
        let hex = |b: &[u8]| {
            let mut t: String = b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
            if (b.len() as u64) < self.len {
                t.push_str(" …");
            }
            t
        };
        let what = match (&self.label, self.place()) {
            (Some(l), Some(p)) => format!("{l} ({p})"),
            (Some(l), None) => l.clone(),
            (None, Some(p)) => p,
            (None, None) => "(no name)".into(),
        };
        if self.expected.is_empty() && self.actual.is_empty() {
            what
        } else {
            format!("{what}: {} here, {} in the original", hex(&self.actual), hex(&self.expected))
        }
    }
}

impl Diffs {
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Up to `n` lines describing them, and a line counting the rest.
    pub fn lines(&self, n: usize) -> Vec<String> {
        let mut v: Vec<String> = self.list.iter().take(n).map(Diff::describe).collect();
        let rest = self.total.saturating_sub(v.len() as u64);
        if rest > 0 {
            v.push(format!("and {rest} more"));
        }
        v
    }

    /// The differences between two copies of a region, one per run of differing bytes.
    pub fn of_bytes(region: &str, expected: &[u8], actual: &[u8]) -> Diffs {
        let mut d = Diffs::default();
        let mut i = 0;
        let n = expected.len().min(actual.len());
        while i < n {
            if expected[i] == actual[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < n && expected[i] != actual[i] {
                i += 1;
            }
            d.total += 1;
            if d.list.len() < DIFF_CAP {
                let end = i.min(start + DIFF_BYTES);
                d.list.push(Diff { region: Some(region.to_string()), address: Some(start as u64), len: (i - start) as u64, expected: expected[start..end].to_vec(), actual: actual[start..end].to_vec(), ..Default::default() });
            }
        }
        d
    }
}

// ---- events (.agents/yield-parity/PLAN.md, sections 1 to 4) ---------------------------------

/// `struct dj_baseline`: what a comparison core reads the baseline through during an event.
/// Memory and registers, and nothing else.
#[repr(C)]
struct DjBaseline {
    version: u32,
    ctx: *mut c_void,
    memory: unsafe extern "C" fn(*mut c_void, *const c_char, *mut usize) -> *const u8,
    registers: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> usize,
}

/// `struct dj_breakpoint`: a place in the original where the core wants to be called.
#[repr(C)]
struct DjBreakpointC {
    id: u32,
    region: *const c_char,
    offset: u64,
    name: *const c_char,
}

/// `struct dj_event`.
#[repr(C)]
struct DjEventC {
    size: u32,
    kind: u32,
    flags: u32,
    breakpoint: u32,
    jump_reason: u32,
    segment: *const c_char,
    frame: u64,
    baseline: *const DjBaseline,
}

/// `struct dj_write`.
#[repr(C)]
struct DjWriteC {
    size: u32,
    flags: u32,
    segment: *const c_char,
    frame: u64,
    baseline: *const DjBaseline,
    region: *const c_char,
    offset: u64,
    bytes: *const u8,
    len: usize,
    at_region: *const c_char,
    at_offset: u64,
}

const DJ_EVENT_JUMP: u32 = 1;
const DJ_EVENT_CONSOLE_RESET: u32 = 2;
const DJ_EVENT_BREAK: u32 = 3;
const DJ_EVENT_FRAME_END: u32 = 4;
/// In `flags`: the frame is pre-roll.
const DJ_PREROLL: u32 = 1;

type EventFn = unsafe extern "C" fn(*const DjEventC, *mut c_char, usize) -> i32;
type WriteEventFn = unsafe extern "C" fn(*const DjWriteC, *mut c_char, usize) -> i32;
type BreakpointsFn = unsafe extern "C" fn(*mut *const DjBreakpointC) -> usize;

/// Why the baseline's state is not what the comparison core last saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpReason {
    /// A fresh power-on.
    PowerOn = 1,
    /// A state was loaded: a seek, a snapshot, a segment's start.
    State = 2,
    /// Nothing was replaced; the core is asked to copy the baseline again (after a halt, a
    /// write it can't follow, Sync now, or being switched on).
    CopyAgain = 3,
}

/// One call to a comparison core's `dj_handle_event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Jump(JumpReason),
    /// A `[RESET]` line was applied to the baseline.
    ConsoleReset,
    /// The baseline reached the core's breakpoint with this id.
    Break(u32),
    FrameEnd,
}

/// What a comparison core answers to an event (`enum dj_verdict`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Pending,
    Match,
    Mismatch,
    Unjudged,
    Halt,
}

/// An answer with its reason, and what differed (for a mismatch or a halt).
#[derive(Clone, Debug)]
pub struct Reply {
    pub answer: Answer,
    pub reason: String,
    pub diffs: Diffs,
}

/// A place in the original where a comparison core wants to be called (`dj_breakpoints`).
#[derive(Clone, Debug, PartialEq)]
pub struct Breakpoint {
    pub id: u32,
    /// The baseline connector's region: "PRG", "RDRAM".
    pub region: String,
    pub offset: u64,
    pub name: String,
}

unsafe extern "C" fn baseline_memory(ctx: *mut c_void, name: *const c_char, size: *mut usize) -> *const u8 {
    if !size.is_null() {
        *size = 0;
    }
    if ctx.is_null() || name.is_null() {
        return std::ptr::null();
    }
    let core = &*(ctx as *const Core);
    let name = CStr::from_ptr(name).to_string_lossy().to_ascii_uppercase();
    match crate::libretro::region_memory_id(&name).and_then(|id| core.memory_ptr(id)) {
        Some((ptr, len)) => {
            if !size.is_null() {
                *size = len;
            }
            ptr
        }
        None => std::ptr::null(),
    }
}

unsafe extern "C" fn baseline_registers(ctx: *mut c_void, out: *mut c_void, cap: usize) -> usize {
    if ctx.is_null() || out.is_null() {
        return 0;
    }
    let core = &*(ctx as *const Core);
    match core.registers() {
        Some(r) if r.len() <= cap => {
            std::ptr::copy_nonoverlapping(r.as_ptr(), out as *mut u8, r.len());
            r.len()
        }
        _ => 0,
    }
}

fn c_text(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Reads the differences from a core's `dj_parity_diffs`.
fn read_diffs(f: DiffsFn) -> Diffs {
    let empty = DjParityDiff { label: std::ptr::null(), region: std::ptr::null(), address: 0, symbol: std::ptr::null(), expected: std::ptr::null(), actual: std::ptr::null(), len: 0 };
    let mut buf = vec![empty; DIFF_CAP];
    let total = unsafe { f(buf.as_mut_ptr(), buf.len()) };
    let bytes = |p: *const u8, len: usize| if p.is_null() { vec![] } else { unsafe { std::slice::from_raw_parts(p, len.min(DIFF_BYTES)) }.to_vec() };
    let list = buf[..total.min(DIFF_CAP)]
        .iter()
        .map(|d| {
            let region = c_text(d.region);
            let symbol = c_text(d.symbol);
            Diff { label: c_text(d.label), address: region.is_some().then_some(d.address), region, symbol, len: d.len as u64, expected: bytes(d.expected, d.len), actual: bytes(d.actual, d.len) }
        })
        .collect();
    Diffs { list, total: total as u64 }
}
fn reason(buf: &[u8]) -> Option<String> {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// What a sync (`sync = "state"`) came to.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncOutcome {
    Synced,
    /// The baseline's state doesn't load in the core, with the reason.
    Failed(String),
}

/// One frame the comparison core ran.
pub struct Stepped {
    pub frame: Option<Frame>,
}

/// A parity check of one frame (docs/connectors.md#parity).
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Match,
    /// The cores disagree after this frame, with the reason and what differed (empty when
    /// the core doesn't say).
    Mismatch(String, Diffs),
    /// Not checked: there is nothing to compare, with the reason.
    Skip(String),
}

/// A loaded comparison core.
pub struct Comparison {
    pub name: String,
    pub core: Core,
    pub mode: SyncMode,
    /// `preroll` of the `[[compare]]` entry; 0 for a core that doesn't take events.
    pub preroll: u64,
    diffs_fn: Option<DiffsFn>,
    /// `dj_handle_event`: the core takes events, and runs itself inside them.
    event_fn: Option<EventFn>,
    /// `dj_handle_write` (optional, for a core that takes events).
    write_event_fn: Option<WriteEventFn>,
    /// `dj_breakpoints`: where in the original the core wants to be called.
    breakpoints: Vec<Breakpoint>,
    /// The sha256 of the core library as loaded: the build parity results belong to.
    pub build: String,
    /// A private copy of the library, when it is the baseline's own file.
    copy: Option<PathBuf>,
}

impl Comparison {
    /// Loads comparison core `name` from the project's `[[compare]]` entries. `baseline_core`
    /// is the baseline's library (the same file is loaded from a private copy) and `rom` the
    /// project's ROM.
    pub fn open(col: &Collection, name: &str, baseline_core: &Path, rom: Option<&Path>) -> Result<Comparison> {
        Comparison::open_spec(&OpenSpec::resolve(col, name, baseline_core, rom)?)
    }

    /// Loads a comparison core as `spec` says.
    pub fn open_spec(spec: &OpenSpec) -> Result<Comparison> {
        let (name, path) = (&spec.name, &spec.library);
        std::fs::create_dir_all(&spec.work)?;
        // A library is loaded once per process: the baseline's own file would share its globals.
        let same = std::fs::canonicalize(path).ok().is_some_and(|p| std::fs::canonicalize(&spec.baseline_library).ok() == Some(p));
        let (load_path, copy) = if same {
            let file = path.file_name().ok_or_else(|| anyhow!("bad core path {}", path.display()))?.to_string_lossy().into_owned();
            let copy = spec.work.join(format!("{}-{}", std::process::id(), file));
            std::fs::copy(path, &copy).with_context(|| format!("copying {} to load it a second time", path.display()))?;
            (copy.clone(), Some(copy))
        } else {
            (path.clone(), None)
        };
        let loaded = (|| -> Result<Comparison> {
            let mut core = Core::load_in(COMPARISON, &load_path, &spec.work, &spec.options)?;
            match &spec.content {
                Loads::Rom => {
                    let rom = spec.rom.as_ref().ok_or_else(|| anyhow!("no ROM is set for this project"))?;
                    core.load_game(rom)?;
                }
                Loads::Nothing => core.load_no_game()?,
                Loads::File(p) => core.load_game(p)?,
            }
            let diffs_ptr = core.proc("dj_parity_diffs");
            let diffs_fn: Option<DiffsFn> = (!diffs_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, DiffsFn>(diffs_ptr) });
            let ev_ptr = core.proc("dj_handle_event");
            let event_fn: Option<EventFn> = (!ev_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, EventFn>(ev_ptr) });
            let wev_ptr = core.proc("dj_handle_write");
            let write_event_fn: Option<WriteEventFn> = (!wev_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, WriteEventFn>(wev_ptr) });
            let bp_ptr = core.proc("dj_breakpoints");
            let mut breakpoints = vec![];
            if event_fn.is_some() && !bp_ptr.is_null() {
                let f: BreakpointsFn = unsafe { std::mem::transmute(bp_ptr) };
                let mut list: *const DjBreakpointC = std::ptr::null();
                let n = core.in_core(crate::crash::CALL_DJ_BREAKPOINTS, || unsafe { f(&mut list) });
                if !list.is_null() {
                    for b in unsafe { std::slice::from_raw_parts(list, n) } {
                        let region = c_text(b.region).ok_or_else(|| anyhow!("dj_breakpoints: breakpoint {} names no region", b.id))?.to_ascii_uppercase();
                        breakpoints.push(Breakpoint { id: b.id, region, offset: b.offset, name: c_text(b.name).unwrap_or_else(|| format!("breakpoint {}", b.id)) });
                    }
                }
            }
            if event_fn.is_some() {
                // It draws when it chooses: tell it whether a picture is wanted.
                core.set_video_hint(true);
            }
            match spec.mode {
                SyncMode::Events if event_fn.is_none() => bail!("the core has no dj_handle_event, which [[compare]] \"{name}\" needs (sync = \"events\", the default: docs/connectors.md, \"Comparison cores\"). For the same emulator as the baseline, sync = \"state\" follows it by save state instead"),
                SyncMode::State => {
                    core.serialize().context("sync = \"state\" needs save states, and this core can't save one")?;
                }
                _ => {}
            }
            Ok(Comparison { name: name.to_string(), core, mode: spec.mode, preroll: if spec.mode == SyncMode::Events { spec.preroll } else { 0 }, diffs_fn, event_fn, write_event_fn, breakpoints, build: spec.build.clone(), copy: None })
        })();
        match loaded {
            Ok(mut c) => {
                c.copy = copy;
                // It shares dj's process: a crash note names it (crash.rs).
                crate::crash::set_compare(&format!("{} ({})", c.name, path.display()));
                Ok(c)
            }
            Err(e) => {
                if let Some(c) = copy {
                    let _ = std::fs::remove_file(c);
                }
                Err(e)
            }
        }
    }

    /// Brings a `sync = "state"` core to the baseline's state at the start of frame `frame`
    /// of segment `id`, by loading the baseline's save state. An error is a sync that went
    /// wrong (as opposed to a state that doesn't load). A core that takes events is not
    /// synced: it is sent a jump ([`crate::parity::Checker::sync`]).
    pub fn sync(&mut self, baseline: &mut Core, col: &Collection, id: &str, frame: u64) -> Result<SyncOutcome> {
        self.sync_with(baseline, &InputState::at(col, id, frame)?)
    }

    /// `sync`, with the inputs held at that frame given.
    pub fn sync_with(&mut self, baseline: &mut Core, inputs: &InputState) -> Result<SyncOutcome> {
        if self.mode != SyncMode::State {
            bail!("a comparison core that takes events is not synced by dj");
        }
        let state = baseline.serialize().context("saving the baseline's state")?;
        let out = match self.core.unserialize(&state) {
            Ok(()) => SyncOutcome::Synced,
            Err(e) => SyncOutcome::Failed(format!("{e} (the baseline's save state doesn't load in this core)")),
        };
        if out == SyncOutcome::Synced {
            inputs.apply(&mut self.core);
        }
        Ok(out)
    }

    /// Sets devices and releases inputs at frame 0 of `id`, as the baseline does when
    /// playback carries straight on into a child segment.
    pub fn enter(&mut self, col: &Collection, id: &str) -> Result<()> {
        restore_inputs_on(&mut self.core, col, id, 0)
    }

    /// Applies frame `frame`'s lines of `id` and runs that frame, for a `sync = "state"` core.
    pub fn step(&mut self, col: &Collection, id: &str, frame: u64, capture: bool) -> Result<Stepped> {
        self.step_with(&Action::of_frame(col, id, frame), capture)
    }

    /// Whether the core takes events (`dj_handle_event`): dj then calls it at each jump,
    /// console reset, breakpoint and frame end, and the core runs itself inside the call.
    pub fn takes_events(&self) -> bool {
        self.event_fn.is_some() && self.mode == SyncMode::Events
    }

    /// Where in the original the core wants to be called.
    pub fn breakpoints(&self) -> &[Breakpoint] {
        &self.breakpoints
    }

    /// Whether the core follows `[WRITE]` lines itself (`dj_handle_write`).
    pub fn handles_writes(&self) -> bool {
        self.write_event_fn.is_some()
    }

    fn handle(baseline: *const Core) -> DjBaseline {
        DjBaseline { version: 2, ctx: baseline as *mut c_void, memory: baseline_memory, registers: baseline_registers }
    }

    /// The core's answer `r`, with its reason, and what differed for a mismatch or a halt.
    fn reply(&mut self, r: i32, buf: &[u8], call: &str) -> Reply {
        let (answer, fallback) = match r {
            0 => (Answer::Pending, None),
            1 => (Answer::Match, None),
            2 => (Answer::Mismatch, Some("the core reports a mismatch".to_string())),
            3 => (Answer::Unjudged, Some("not judged here".to_string())),
            4 => (Answer::Halt, Some("the core can't go on".to_string())),
            other => (Answer::Halt, Some(format!("{call} returned {other}, which is no verdict"))),
        };
        let reason = reason(buf).or(fallback).unwrap_or_default();
        let diffs = match (answer, self.diffs_fn) {
            (Answer::Mismatch | Answer::Halt, Some(f)) => {
                let _call = crate::crash::CompareCall::enter(crate::crash::CALL_DJ_PARITY_DIFFS);
                read_diffs(f)
            }
            _ => Diffs::default(),
        };
        Reply { answer, reason, diffs }
    }

    /// Calls the core's `dj_handle_event`. `baseline` is the baseline core, stopped: between
    /// frames, or inside its frame at a breakpoint. The core may read it through the handle
    /// for the length of the call.
    ///
    /// # Safety
    /// `baseline` must point at the live baseline core.
    pub unsafe fn event(&mut self, baseline: *const Core, ev: Event, segment: &str, frame: u64, preroll: bool) -> Reply {
        let Some(f) = self.event_fn else { return Reply { answer: Answer::Pending, reason: String::new(), diffs: Diffs::default() } };
        let h = Self::handle(baseline);
        let seg = CString::new(segment).unwrap_or_default();
        let (kind, breakpoint, jump_reason) = match ev {
            Event::Jump(r) => (DJ_EVENT_JUMP, 0, r as u32),
            Event::ConsoleReset => (DJ_EVENT_CONSOLE_RESET, 0, 0),
            Event::Break(id) => (DJ_EVENT_BREAK, id, 0),
            Event::FrameEnd => (DJ_EVENT_FRAME_END, 0, 0),
        };
        let c = DjEventC { size: std::mem::size_of::<DjEventC>() as u32, kind, flags: if preroll { DJ_PREROLL } else { 0 }, breakpoint, jump_reason, segment: seg.as_ptr(), frame, baseline: &h };
        let mut buf = vec![0u8; 512];
        let r = self.core.in_core(crate::crash::CALL_DJ_HANDLE_EVENT, || f(&c, buf.as_mut_ptr() as *mut c_char, buf.len()));
        self.reply(r, &buf, "dj_handle_event")
    }

    /// Calls the core's `dj_handle_write`: a `[WRITE]` line was applied to the baseline.
    /// `at`: the instruction it was made at, for a line that names one. A core without the
    /// function answers nothing (pending); the caller then asks it to copy the baseline again.
    ///
    /// # Safety
    /// `baseline` must point at the live baseline core.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn write_event(&mut self, baseline: *const Core, segment: &str, frame: u64, preroll: bool, region: &str, offset: u64, bytes: &[u8], at: Option<(&str, u64)>) -> Reply {
        let Some(f) = self.write_event_fn else { return Reply { answer: Answer::Pending, reason: String::new(), diffs: Diffs::default() } };
        let h = Self::handle(baseline);
        let seg = CString::new(segment).unwrap_or_default();
        let region_c = CString::new(region).unwrap_or_default();
        let at_c = at.map(|(r, _)| CString::new(r).unwrap_or_default());
        let c = DjWriteC {
            size: std::mem::size_of::<DjWriteC>() as u32,
            flags: if preroll { DJ_PREROLL } else { 0 },
            segment: seg.as_ptr(),
            frame,
            baseline: &h,
            region: region_c.as_ptr(),
            offset,
            bytes: bytes.as_ptr(),
            len: bytes.len(),
            at_region: at_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
            at_offset: at.map_or(0, |(_, o)| o),
        };
        let mut buf = vec![0u8; 512];
        let r = self.core.in_core(crate::crash::CALL_DJ_HANDLE_WRITE, || f(&c, buf.as_mut_ptr() as *mut c_char, buf.len()));
        self.reply(r, &buf, "dj_handle_write")
    }

    /// `step`, with the frame's lines given as `actions`.
    pub fn step_with(&mut self, actions: &[Action], capture: bool) -> Result<Stepped> {
        if self.mode != SyncMode::State {
            bail!("a comparison core that takes events runs itself");
        }
        let mut impulses = vec![];
        for action in actions {
            match action {
                Action::Input { key, value, impulse } => {
                    self.core.set_input(*key, *value);
                    if *impulse {
                        impulses.push(*key);
                    }
                }
                Action::Device { port, device } => self.core.set_port_device(*port, *device),
                // The same emulator: the write goes straight into its memory, or, for a line
                // that names an instruction, is made when this core runs it too.
                Action::Write { region, address, bytes, at: None } => write_memory(&mut self.core, region, *address, bytes)?,
                Action::Write { region, address, bytes, at: Some((at_region, at_offset)) } => self.core.arm_write(at_region, *at_offset, region, *address, bytes, &format!("[WRITE] {region} {address:#X} @{at_region}:{at_offset:#X} (comparison core)"))?,
                Action::Reset => self.core.reset(),
                Action::Rng => bail!("[RNG] is not supported yet"),
            }
        }
        let frame = self.core.run_frame(capture);
        for k in impulses {
            self.core.set_input(k, 0);
        }
        Ok(Stepped { frame })
    }

    /// Checks parity after a frame both cores ran from a synced state, for a `sync = "state"`
    /// core: the two cores' system RAM, byte for byte.
    pub fn check(&mut self, baseline: &mut Core) -> Result<Verdict> {
        let base = baseline.memory_view(MEMORY_SYSTEM_RAM).map(|m| m.to_vec());
        let mine = self.core.memory(MEMORY_SYSTEM_RAM);
        Ok(match (base, mine) {
            (Some(a), Some(b)) if a.len() == b.len() => match a.iter().zip(b.iter()).position(|(x, y)| x != y) {
                None => Verdict::Match,
                Some(i) => {
                    let d = Diffs::of_bytes("RAM", &a, b);
                    let more = if d.total > 1 { format!(" ({} ranges differ)", d.total) } else { String::new() };
                    Verdict::Mismatch(format!("system RAM differs at 0x{i:X}: 0x{:02X} here, 0x{:02X} in the baseline{more}", b[i], a[i]), d)
                }
            },
            (Some(a), Some(b)) => Verdict::Skip(format!("system RAM sizes differ ({} bytes here, {} in the baseline)", b.len(), a.len())),
            _ => Verdict::Skip("no system RAM to compare".into()),
        })
    }

    /// The core's preferred display aspect ratio (0 if unset).
    pub fn aspect(&self) -> f32 {
        self.core.timing().1
    }

}

impl Drop for Comparison {
    fn drop(&mut self) {
        crate::crash::set_compare("");
        if let Some(c) = &self.copy {
            // Unix keeps a loaded library's file alive after it is removed.
            let _ = std::fs::remove_file(c);
        }
    }
}

/// What a comparison core loads.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Loads {
    /// The project's ROM.
    Rom,
    Nothing,
    File(PathBuf),
}

/// Everything needed to load a comparison core, resolved from the project.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenSpec {
    pub name: String,
    pub library: PathBuf,
    /// The library's sha256: the build parity results belong to.
    pub build: String,
    pub mode: SyncMode,
    #[serde(default)]
    pub preroll: u64,
    pub content: Loads,
    pub options: BTreeMap<String, String>,
    /// `.local/compare/<name>/`: the core's save and system folders.
    pub work: PathBuf,
    /// The baseline core's library: a comparison core can't be the same file.
    pub baseline_library: PathBuf,
    pub rom: Option<PathBuf>,
}

impl OpenSpec {
    /// Resolves `[[compare]]` entry `name`, checking the library exists and matches its pin.
    pub fn resolve(col: &Collection, name: &str, baseline_core: &Path, rom: Option<&Path>) -> Result<OpenSpec> {
        let cfg = col.config.compare.iter().find(|c| c.name == name).ok_or_else(|| anyhow!("no [[compare]] entry is named \"{name}\""))?;
        let path = cfg.path.as_ref().ok_or_else(|| anyhow!("[[compare]] \"{name}\" has no path: set `path` (in config.toml, or in .local/local.toml for this machine)"))?;
        let path = crate::libretro::library_path(&if path.is_relative() { col.root.join(path) } else { path.clone() });
        if !path.is_file() {
            bail!("the core library {} doesn't exist", path.display());
        }
        let build = sha256_file(&path)?;
        if !cfg.sha256.is_empty() && build != cfg.sha256 {
            bail!("core {} has sha256 {build}, but [[compare]] \"{name}\" is pinned to {}", path.display(), cfg.sha256);
        }
        let content = match &cfg.content {
            None | Some(Content::Rom(true)) => Loads::Rom,
            Some(Content::Rom(false)) => Loads::Nothing,
            Some(Content::Path(p)) => Loads::File(if p.is_relative() { col.root.join(p) } else { p.clone() }),
        };
        Ok(OpenSpec {
            name: name.to_string(),
            library: path,
            build,
            mode: cfg.sync,
            preroll: cfg.preroll,
            content,
            options: cfg.options.clone(),
            work: col.local_dir().join("compare").join(sanitize(name)),
            baseline_library: crate::libretro::library_path(baseline_core),
            rom: rom.map(Path::to_path_buf),
        })
    }
}

/// The inputs held and devices plugged in at a frame: what a sync leaves the comparison
/// core with (`restore_inputs_on`, as data).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct InputState {
    pub devices: Vec<(u32, u32)>,
    pub held: Vec<(InputKey, i16)>,
}

impl InputState {
    pub fn at(col: &Collection, id: &str, frame: u64) -> Result<InputState> {
        let mut devs = crate::player::devices_at_start(col, id)?;
        let mut held: BTreeMap<InputKey, i16> = BTreeMap::new();
        for l in col.segments[id].lines.iter().take_while(|l| l.frame < frame) {
            match &l.command {
                Command::Input { key, value, impulse: false } => {
                    held.insert(*key, *value);
                }
                Command::Device { port, device } => {
                    devs.insert(*port, *device);
                }
                _ => {}
            }
        }
        Ok(InputState { devices: devs.into_iter().collect(), held: held.into_iter().collect() })
    }

    pub fn apply(&self, core: &mut Core) {
        core.clear_inputs();
        for (key, value) in &self.held {
            core.set_input(*key, *value);
        }
        for (port, dev) in &self.devices {
            core.set_port_device(*port, *dev);
        }
    }
}

/// One frame's lines, as a comparison core follows them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Action {
    Input { key: InputKey, value: i16, impulse: bool },
    Device { port: u32, device: u32 },
    /// `at`: the instruction the write is made at, for a line that names one.
    Write { region: String, address: u64, bytes: Vec<u8>, at: Option<(String, u64)> },
    Reset,
    Rng,
}

impl Action {
    /// The lines of frame `frame` of segment `id` a comparison core acts on.
    pub fn of_frame(col: &Collection, id: &str, frame: u64) -> Vec<Action> {
        let r = &col.segments[id];
        let first = r.lines.partition_point(|l| l.frame < frame);
        r.lines[first..]
            .iter()
            .take_while(|l| l.frame == frame)
            .filter_map(|l| match &l.command {
                Command::Input { key, value, impulse } => Some(Action::Input { key: *key, value: *value, impulse: *impulse }),
                Command::Device { port, device } => Some(Action::Device { port: *port, device: *device }),
                Command::Write { region, address, bytes, at } => Some(Action::Write { region: region.clone(), address: *address, bytes: bytes.clone(), at: at.clone() }),
                Command::Reset => Some(Action::Reset),
                Command::Rng { .. } => Some(Action::Rng),
                Command::Event { .. } | Command::Screenshot => None,
            })
            .collect()
    }
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A diff with a symbol keeps its address: the symbol is for reading, the address is
    /// what `dj parity explain` traces.
    #[test]
    fn a_symbol_goes_beside_the_address() {
        let d = Diff { region: Some("RAM".into()), address: Some(0x45), symbol: Some("fallTimer".into()), len: 1, ..Default::default() };
        assert_eq!(d.place().as_deref(), Some("fallTimer, RAM 0x0045"));
        let d = Diff { symbol: Some("fallTimer".into()), len: 1, ..Default::default() };
        assert_eq!(d.place().as_deref(), Some("fallTimer"));
    }

    #[test]
    fn byte_diffs_are_runs() {
        let d = Diffs::of_bytes("RAM", &[1, 2, 3, 4, 5, 6], &[1, 9, 9, 4, 5, 0]);
        assert_eq!(d.total, 2);
        assert_eq!((d.list[0].address, d.list[0].len, d.list[0].expected.clone(), d.list[0].actual.clone()), (Some(1), 2, vec![2, 3], vec![9, 9]));
        assert_eq!((d.list[1].address, d.list[1].len), (Some(5), 1));
        assert!(Diffs::of_bytes("RAM", &[1], &[1]).is_empty());
    }

    #[test]
    fn parses_compare_entries() {
        let t: toml::Table = toml::from_str(
            "[[compare]]\nname = \"port\"\npath = \"cores/port.so\"\ncontent = false\n[compare.options]\nx = \"1\"\n\
             [[compare]]\nname = \"mesen-new\"\nsync = \"state\"\ncontent = \"other.nes\"\n\
             [[compare]]\nname = \"plain\"\n",
        )
        .unwrap();
        #[derive(Deserialize)]
        struct W {
            compare: Vec<CompareConfig>,
        }
        let w: W = toml::Value::Table(t).try_into().unwrap();
        assert_eq!(w.compare[0].sync, SyncMode::Events);
        assert_eq!(w.compare[0].content, Some(Content::Rom(false)));
        assert_eq!(w.compare[0].options.get("x").map(String::as_str), Some("1"));
        assert_eq!(w.compare[1].sync, SyncMode::State);
        assert_eq!(w.compare[1].content, Some(Content::Path("other.nes".into())));
        assert_eq!(w.compare[2].content, None);
        assert!(toml::from_str::<W>("[[compare]]\nname = \"x\"\nsync = \"nope\"\n").is_err());
    }
}
