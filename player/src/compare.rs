//! Comparison cores: a second libretro core that plays beside the baseline, driven by the
//! same input lines (docs/config.md, docs/connectors.md#comparison-cores).
//!
//! After a jump, the comparison core is brought to the baseline's state with a sync:
//! either its own `dj_sync`, which gets a handle to the baseline core, or (for the same
//! emulator) by loading the baseline's save state.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use crate::collection::{sha256_file, Collection};
use crate::format::{Command, InputKey};
use crate::libretro::{Core, Frame, COMPARISON, MEMORY_SYSTEM_RAM};
use crate::player::{restore_inputs_on, write_memory};

/// How a comparison core is brought to the baseline's state after a jump.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncMode {
    /// The core's own `dj_sync`, which must exist.
    #[default]
    #[serde(rename = "dj_sync")]
    DjSync,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
}

/// `struct dj_baseline` (docs/connectors.md): the baseline core, as a comparison core sees it.
#[repr(C)]
struct DjBaseline {
    version: u32,
    ctx: *mut c_void,
    get_proc: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
}

const DJ_BASELINE_VERSION: u32 = 1;
pub const DJ_SYNC_BOOT: u32 = 1;
const DJ_SYNC_OK: i32 = 0;
const DJ_SYNC_FAILED: i32 = 1;
const DJ_WRITE_OK: i32 = 0;
const DJ_WRITE_IGNORED: i32 = 1;

const DJ_PARITY_MATCH: i32 = 0;
const DJ_PARITY_MISMATCH: i32 = 1;
const DJ_PARITY_SKIP: i32 = 2;
const DJ_RUN_OK: i32 = 0;

type CheckFn = unsafe extern "C" fn(*const DjBaseline, u64, *mut c_char, usize) -> i32;
type DiffsFn = unsafe extern "C" fn(*mut DjParityDiff, usize) -> usize;

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
            Diff { label: c_text(d.label), address: (region.is_some() && symbol.is_none()).then_some(d.address), region, symbol, len: d.len as u64, expected: bytes(d.expected, d.len), actual: bytes(d.actual, d.len) }
        })
        .collect();
    Diffs { list, total: total as u64 }
}
type RunFn = unsafe extern "C" fn(*mut c_char, usize) -> i32;
type SyncFn = unsafe extern "C" fn(*const DjBaseline, u64, u32, *mut c_char, usize) -> i32;
type WriteFn = unsafe extern "C" fn(*const DjBaseline, *const c_char, u64, *const u8, usize, *mut c_char, usize) -> i32;

unsafe extern "C" fn baseline_get_proc(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if ctx.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let core = &*(ctx as *const Core);
    core.proc(&CStr::from_ptr(name).to_string_lossy())
}

fn handle(baseline: &Core) -> DjBaseline {
    DjBaseline { version: DJ_BASELINE_VERSION, ctx: baseline as *const Core as *mut c_void, get_proc: baseline_get_proc }
}

fn reason(buf: &[u8]) -> Option<String> {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// What a sync came to.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncOutcome {
    Synced,
    /// The core says it can't match the baseline here, with its reason.
    Failed(String),
}

/// One frame the comparison core ran.
pub struct Stepped {
    pub frame: Option<Frame>,
    /// The frame had a line the comparison couldn't follow (a `[WRITE]` without
    /// `dj_write`): sync it again before trusting it.
    pub needs_sync: bool,
    /// The core's `dj_run` failed this frame, with its reason.
    pub tick_failed: Option<String>,
}

/// A parity check of one frame (docs/connectors.md#parity).
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Match,
    /// The cores disagree after this frame, with the reason and what differed (empty when
    /// the core doesn't say).
    Mismatch(String, Diffs),
    /// Not checked: the core can't judge this frame (a loading screen, say), with the reason.
    Skip(String),
}

/// A loaded comparison core.
pub struct Comparison {
    pub name: String,
    pub core: Core,
    pub mode: SyncMode,
    sync_fn: Option<SyncFn>,
    write_fn: Option<WriteFn>,
    check_fn: Option<CheckFn>,
    diffs_fn: Option<DiffsFn>,
    run_fn: Option<RunFn>,
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
            let sync_ptr = core.proc("dj_sync");
            let write_ptr = core.proc("dj_write");
            let sync_fn: Option<SyncFn> = (!sync_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, SyncFn>(sync_ptr) });
            let write_fn: Option<WriteFn> = (!write_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, WriteFn>(write_ptr) });
            let check_ptr = core.proc("dj_check_parity");
            let run_ptr = core.proc("dj_run");
            let check_fn: Option<CheckFn> = (!check_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, CheckFn>(check_ptr) });
            let diffs_ptr = core.proc("dj_parity_diffs");
            let diffs_fn: Option<DiffsFn> = (!diffs_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, DiffsFn>(diffs_ptr) });
            let run_fn: Option<RunFn> = (!run_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, RunFn>(run_ptr) });
            match spec.mode {
                SyncMode::DjSync if sync_fn.is_none() => bail!("the core has no dj_sync, which [[compare]] \"{name}\" asks for (sync = \"dj_sync\", the default). For the same emulator as the baseline, sync = \"state\" syncs by save state instead"),
                SyncMode::State => {
                    core.serialize().context("sync = \"state\" needs save states, and this core can't save one")?;
                }
                _ => {}
            }
            Ok(Comparison { name: name.to_string(), core, mode: spec.mode, sync_fn, write_fn, check_fn, diffs_fn, run_fn, build: spec.build.clone(), copy: None })
        })();
        match loaded {
            Ok(mut c) => {
                c.copy = copy;
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

    /// Brings the comparison core to the baseline's state at the start of frame `frame` of
    /// segment `id`. `power_on_frame` is the same frame counted from power-on, which is what
    /// `dj_sync` is told. `boot` marks a fresh power-on (frame 0 of a root segment).
    /// An error is a sync that went wrong (as opposed to one the core declined).
    pub fn sync(&mut self, baseline: &mut Core, col: &Collection, id: &str, frame: u64, power_on_frame: u64, boot: bool) -> Result<SyncOutcome> {
        self.sync_with(baseline, power_on_frame, boot, &InputState::at(col, id, frame)?)
    }

    /// `sync`, with the inputs held at that frame given.
    pub fn sync_with(&mut self, baseline: &mut Core, power_on_frame: u64, boot: bool, inputs: &InputState) -> Result<SyncOutcome> {
        let out = match self.mode {
            SyncMode::State => {
                let state = baseline.serialize().context("saving the baseline's state")?;
                match self.core.unserialize(&state) {
                    Ok(()) => SyncOutcome::Synced,
                    Err(e) => SyncOutcome::Failed(format!("{e} (the baseline's save state doesn't load in this core)")),
                }
            }
            SyncMode::DjSync => {
                let f = self.sync_fn.ok_or_else(|| anyhow!("the core has no dj_sync"))?;
                let h = handle(baseline);
                let mut buf = vec![0u8; 512];
                let r = unsafe { f(&h, power_on_frame, if boot { DJ_SYNC_BOOT } else { 0 }, buf.as_mut_ptr() as *mut c_char, buf.len()) };
                match r {
                    DJ_SYNC_OK => SyncOutcome::Synced,
                    DJ_SYNC_FAILED => SyncOutcome::Failed(reason(&buf).unwrap_or_else(|| "the core couldn't match the baseline here".into())),
                    _ => bail!("dj_sync failed: {}", reason(&buf).unwrap_or_else(|| format!("error {r}"))),
                }
            }
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

    /// `enter`, with the inputs held at frame 0 given.
    pub fn enter_with(&mut self, inputs: &InputState) {
        inputs.apply(&mut self.core);
    }

    /// Applies frame `frame`'s lines of `id` and runs that frame. `baseline` is handed to
    /// `dj_write` for `[WRITE]` lines.
    pub fn step(&mut self, baseline: &Core, col: &Collection, id: &str, frame: u64, capture: bool) -> Result<Stepped> {
        self.step_with(Some(baseline), &Action::of_frame(col, id, frame), capture)
    }

    /// Whether a frame's `actions` need the baseline (a `[WRITE]` for `dj_write`).
    pub fn needs_baseline(&self, actions: &[Action]) -> bool {
        self.mode == SyncMode::DjSync && self.write_fn.is_some() && actions.iter().any(|a| matches!(a, Action::Write { .. }))
    }

    /// `step`, with the frame's lines given as `actions`. `baseline` is needed only for
    /// `[WRITE]` lines that go to `dj_write`.
    pub fn step_with(&mut self, baseline: Option<&Core>, actions: &[Action], capture: bool) -> Result<Stepped> {
        let mut impulses = vec![];
        let mut needs_sync = false;
        for action in actions {
            match action {
                Action::Input { key, value, impulse } => {
                    self.core.set_input(*key, *value);
                    if *impulse {
                        impulses.push(*key);
                    }
                }
                Action::Device { port, device } => self.core.set_port_device(*port, *device),
                Action::Write { region, address, bytes } => match (self.mode, self.write_fn) {
                    // The same emulator: the write goes straight into its memory.
                    (SyncMode::State, _) => write_memory(&mut self.core, region, *address, bytes)?,
                    (SyncMode::DjSync, Some(f)) => {
                        let baseline = baseline.ok_or_else(|| anyhow!("a [WRITE] for dj_write needs the baseline"))?;
                        let h = handle(baseline);
                        let region_c = CString::new(region.as_str())?;
                        let mut buf = vec![0u8; 512];
                        let r = unsafe { f(&h, region_c.as_ptr(), *address, bytes.as_ptr(), bytes.len(), buf.as_mut_ptr() as *mut c_char, buf.len()) };
                        match r {
                            DJ_WRITE_OK | DJ_WRITE_IGNORED => {}
                            _ => bail!("dj_write failed: {}", reason(&buf).unwrap_or_else(|| format!("error {r}"))),
                        }
                    }
                    // No way to follow the write: sync again after this frame.
                    (SyncMode::DjSync, None) => needs_sync = true,
                },
                Action::Reset => self.core.reset(),
                Action::Rng => bail!("[RNG] is not supported yet"),
            }
        }
        let (out, tick_failed) = match self.run_fn {
            Some(f) => {
                let mut buf = vec![0u8; 512];
                let (out, r) = self.core.run_frame_with(capture, || unsafe { f(buf.as_mut_ptr() as *mut c_char, buf.len()) });
                (out, (r != DJ_RUN_OK).then(|| reason(&buf).unwrap_or_else(|| format!("dj_run returned {r}"))))
            }
            None => (self.core.run_frame(capture), None),
        };
        for k in impulses {
            self.core.set_input(k, 0);
        }
        Ok(Stepped { frame: out, needs_sync, tick_failed })
    }

    /// Checks parity after a frame both cores ran from a synced state: the core's own
    /// `dj_check_parity` when it has one, otherwise the two cores' system RAM byte for byte.
    /// `power_on_frame` is the frame just run, counted from power-on. An error is a check
    /// that went wrong (a failure too, for the player).
    pub fn check(&mut self, baseline: &mut Core, power_on_frame: u64) -> Result<Verdict> {
        if let Some(f) = self.check_fn {
            let h = handle(baseline);
            let mut buf = vec![0u8; 512];
            let r = unsafe { f(&h, power_on_frame, buf.as_mut_ptr() as *mut c_char, buf.len()) };
            return match r {
                DJ_PARITY_MATCH => Ok(Verdict::Match),
                DJ_PARITY_MISMATCH => Ok(Verdict::Mismatch(reason(&buf).unwrap_or_else(|| "the core reports a mismatch".into()), self.diffs_fn.map(read_diffs).unwrap_or_default())),
                DJ_PARITY_SKIP => Ok(Verdict::Skip(reason(&buf).unwrap_or_else(|| "not checked here".into()))),
                _ => bail!("dj_check_parity failed: {}", reason(&buf).unwrap_or_else(|| format!("error {r}"))),
            };
        }
        let base = baseline.memory(MEMORY_SYSTEM_RAM).map(|m| m.to_vec());
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
            (Some(a), Some(b)) => Verdict::Skip(format!("system RAM sizes differ ({} bytes here, {} in the baseline) and the core has no dj_check_parity", b.len(), a.len())),
            _ => Verdict::Skip("no system RAM to compare, and the core has no dj_check_parity".into()),
        })
    }

    pub fn has_check(&self) -> bool {
        self.check_fn.is_some()
    }

    /// The core's preferred display aspect ratio (0 if unset).
    pub fn aspect(&self) -> f32 {
        self.core.timing().1
    }

    pub fn has_write(&self) -> bool {
        self.write_fn.is_some()
    }
}

impl Drop for Comparison {
    fn drop(&mut self) {
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

/// Everything needed to load a comparison core, resolved from the project: what a
/// comparison worker gets (.agents/ISOLATION_PLAN.md).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenSpec {
    pub name: String,
    pub library: PathBuf,
    /// The library's sha256: the build parity results belong to.
    pub build: String,
    pub mode: SyncMode,
    pub content: Loads,
    pub options: BTreeMap<String, String>,
    /// `.local/compare/<name>/`: the core's save and system folders.
    pub work: PathBuf,
    /// The baseline core and its options, for a worker's copy of it (the handle `dj_sync`
    /// and friends get).
    pub baseline_library: PathBuf,
    pub baseline_options: BTreeMap<String, String>,
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
            content,
            options: cfg.options.clone(),
            work: col.local_dir().join("compare").join(sanitize(name)),
            baseline_library: crate::libretro::library_path(baseline_core),
            baseline_options: col.config.core.options.clone(),
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
        for l in col.replays[id].lines.iter().take_while(|l| l.frame < frame) {
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
    Write { region: String, address: u64, bytes: Vec<u8> },
    Reset,
    Rng,
}

impl Action {
    /// The lines of frame `frame` of segment `id` a comparison core acts on.
    pub fn of_frame(col: &Collection, id: &str, frame: u64) -> Vec<Action> {
        let r = &col.replays[id];
        let first = r.lines.partition_point(|l| l.frame < frame);
        r.lines[first..]
            .iter()
            .take_while(|l| l.frame == frame)
            .filter_map(|l| match &l.command {
                Command::Input { key, value, impulse } => Some(Action::Input { key: *key, value: *value, impulse: *impulse }),
                Command::Device { port, device } => Some(Action::Device { port: *port, device: *device }),
                Command::Write { region, address, bytes } => Some(Action::Write { region: region.clone(), address: *address, bytes: bytes.clone() }),
                Command::Reset => Some(Action::Reset),
                Command::Rng { .. } => Some(Action::Rng),
                Command::Event { .. } | Command::Screenshot => None,
            })
            .collect()
    }
}

/// The comparison worker (`__dj-worker compare`): the comparison core, and a copy of the
/// baseline core that the handle given to `dj_sync`, `dj_write` and `dj_check_parity`
/// points to, loaded with the baseline's state before each of those calls.
#[derive(Default)]
pub struct CompareWorker {
    cmp: Option<Comparison>,
    shadow: Option<Core>,
    /// The shadow's private copy of the library, when it's the comparison core's own file.
    shadow_copy: Option<PathBuf>,
}

impl Drop for CompareWorker {
    fn drop(&mut self) {
        self.cmp = None;
        self.shadow = None;
        if let Some(c) = &self.shadow_copy {
            let _ = std::fs::remove_file(c);
        }
    }
}

fn verdict_json(v: &Verdict) -> serde_json::Value {
    match v {
        Verdict::Match => serde_json::json!({ "match": true }),
        Verdict::Mismatch(w, d) => serde_json::json!({ "mismatch": w, "diffs": d }),
        Verdict::Skip(w) => serde_json::json!({ "skip": w }),
    }
}

impl CompareWorker {
    /// Loads the baseline's copy. When it's the comparison core's own library, both load
    /// from private copies of the file, at paths of their own: macOS hands back an
    /// already-loaded library when the path asked for is its install name, and a library
    /// built with its absolute path as install name would otherwise have its copy given back
    /// for the original (the two cores sharing one instance). Loaded first, as in dj's own
    /// process. Returns the core and its copy's path, to remove.
    fn load_shadow(spec: &OpenSpec) -> Result<(Core, Option<PathBuf>)> {
        let work = spec.work.join("baseline");
        std::fs::create_dir_all(&work)?;
        let same = std::fs::canonicalize(&spec.library).ok().is_some_and(|p| std::fs::canonicalize(&spec.baseline_library).ok() == Some(p));
        let copy = if same {
            let file = spec.baseline_library.file_name().ok_or_else(|| anyhow!("bad core path {}", spec.baseline_library.display()))?.to_string_lossy().into_owned();
            // Copies left by workers that were killed (a file still loaded elsewhere stays).
            for e in std::fs::read_dir(&work).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().ends_with(&format!("-{file}")) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            let copy = work.join(format!("{}-{file}", std::process::id()));
            std::fs::copy(&spec.baseline_library, &copy).with_context(|| format!("copying {} for the baseline's copy", spec.baseline_library.display()))?;
            Some(copy)
        } else {
            None
        };
        let path = copy.as_deref().unwrap_or(&spec.baseline_library);
        let core = (|| -> Result<Core> {
            let mut core = Core::load_in(crate::libretro::BASELINE, path, &work, &spec.baseline_options).context("loading a copy of the baseline core for the comparison core's handle")?;
            match &spec.rom {
                Some(rom) => core.load_game(rom)?,
                None => core.load_no_game()?,
            }
            Ok(core)
        })();
        match core {
            Ok(core) => Ok((core, copy)),
            Err(e) => {
                if let Some(c) = copy {
                    let _ = std::fs::remove_file(c);
                }
                Err(e)
            }
        }
    }

    /// The baseline's copy, with `state` loaded into it.
    fn shadow<'a>(shadow: &'a mut Option<Core>, state: &[u8]) -> Result<&'a mut Core> {
        let core = shadow.as_mut().ok_or_else(|| anyhow!("no copy of the baseline is loaded"))?;
        core.unserialize(state).context("loading the baseline's state into its copy")?;
        Ok(core)
    }

    pub fn request(&mut self, cmd: &str, req: crate::isolate::Request) -> crate::isolate::Reply {
        use serde_json::json;
        let state = req.bulk.first().map(Vec::as_slice);
        if cmd == "open" {
            let spec: OpenSpec = serde_json::from_value(req.args["spec"].clone())?;
            let (shadow, copy) = Self::load_shadow(&spec)?;
            self.shadow = Some(shadow);
            self.shadow_copy = copy;
            let c = Comparison::open_spec(&spec)?;
            let info = json!({ "has_check": c.has_check(), "has_write": c.has_write(), "aspect": c.aspect() });
            self.cmp = Some(c);
            return Ok((info, vec![]));
        }
        let Some(c) = self.cmp.as_mut() else { bail!("no comparison core is loaded") };
        match cmd {
            "sync" => {
                let inputs: InputState = serde_json::from_value(req.args["inputs"].clone())?;
                let shadow = Self::shadow(&mut self.shadow, state.ok_or_else(|| anyhow!("a sync needs the baseline's state"))?)?;
                let out = c.sync_with(shadow, req.args["power_on_frame"].as_u64().unwrap_or(0), req.args["boot"].as_bool().unwrap_or(false), &inputs)?;
                Ok((match out {
                    SyncOutcome::Synced => json!({ "synced": true }),
                    SyncOutcome::Failed(why) => json!({ "failed": why }),
                }, vec![]))
            }
            "enter" => {
                c.enter_with(&serde_json::from_value(req.args["inputs"].clone())?);
                Ok((json!(null), vec![]))
            }
            "step" => {
                let actions: Vec<Action> = serde_json::from_value(req.args["actions"].clone())?;
                let baseline = match state {
                    Some(s) => Some(&*Self::shadow(&mut self.shadow, s)?),
                    None => None,
                };
                let st = c.step_with(baseline, &actions, req.args["capture"].as_bool().unwrap_or(false))?;
                let size = st.frame.as_ref().map(|f| json!({ "width": f.width, "height": f.height }));
                Ok((json!({ "needs_sync": st.needs_sync, "tick_failed": st.tick_failed, "frame": size }), st.frame.map(|f| vec![f.rgb]).unwrap_or_default()))
            }
            "check" => {
                let shadow = Self::shadow(&mut self.shadow, state.ok_or_else(|| anyhow!("a parity check needs the baseline's state"))?)?;
                let v = c.check(shadow, req.args["power_on_frame"].as_u64().unwrap_or(0))?;
                Ok((verdict_json(&v), vec![]))
            }
            other => bail!("unknown comparison request {other}"),
        }
    }
}

/// How long a comparison worker may take: to load, and for anything else.
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A comparison core running in a worker process (.agents/ISOLATION_PLAN.md): the same
/// calls as [`Comparison`], and a crash or hang in the core ends the worker, not dj.
pub struct RemoteComparison {
    pub name: String,
    pub build: String,
    pub mode: SyncMode,
    has_check: bool,
    has_write: bool,
    aspect: f32,
    worker: crate::isolate::Worker,
}

impl RemoteComparison {
    pub fn open(col: &Collection, name: &str, baseline_core: &Path, rom: Option<&Path>) -> Result<RemoteComparison> {
        let spec = OpenSpec::resolve(col, name, baseline_core, rom)?;
        let mut worker = crate::isolate::Worker::spawn("compare")?;
        let (info, _) = worker
            .request("open", serde_json::json!({ "spec": spec }), &[], crate::isolate::timeout(OPEN_TIMEOUT), "while loading")
            .map_err(|why| if worker.dead().is_some() { anyhow!("the comparison core {why}") } else { anyhow!("{why}") })?;
        Ok(RemoteComparison {
            name: spec.name,
            build: spec.build,
            mode: spec.mode,
            has_check: info["has_check"].as_bool().unwrap_or(false),
            has_write: info["has_write"].as_bool().unwrap_or(false),
            aspect: info["aspect"].as_f64().unwrap_or(0.0) as f32,
            worker,
        })
    }

    /// Why the worker is gone (it crashed or hung), once it is: the core must be loaded again.
    pub fn dead(&self) -> Option<&str> {
        self.worker.dead()
    }

    fn call(&mut self, cmd: &str, args: serde_json::Value, bulk: &[&[u8]], what: &str) -> Result<(serde_json::Value, Vec<Vec<u8>>)> {
        self.worker.request(cmd, args, bulk, crate::isolate::timeout(CALL_TIMEOUT), what).map_err(|why| {
            if self.worker.dead().is_some() {
                anyhow!("the comparison core {why}; Compare > Restart comparison core loads it again")
            } else {
                anyhow!("{why}")
            }
        })
    }

    pub fn sync(&mut self, baseline: &mut Core, col: &Collection, id: &str, frame: u64, power_on_frame: u64, boot: bool) -> Result<SyncOutcome> {
        let state = baseline.serialize().context("saving the baseline's state")?;
        let args = serde_json::json!({ "inputs": InputState::at(col, id, frame)?, "power_on_frame": power_on_frame, "boot": boot });
        let what = if self.mode == SyncMode::DjSync { "while syncing (dj_sync)" } else { "while loading the baseline's state" };
        let (v, _) = self.call("sync", args, &[&state], what)?;
        Ok(match v["failed"].as_str() {
            Some(why) => SyncOutcome::Failed(why.to_string()),
            None => SyncOutcome::Synced,
        })
    }

    pub fn enter(&mut self, col: &Collection, id: &str) -> Result<()> {
        self.call("enter", serde_json::json!({ "inputs": InputState::at(col, id, 0)? }), &[], "while entering a segment")?;
        Ok(())
    }

    pub fn step(&mut self, baseline: &mut Core, col: &Collection, id: &str, frame: u64, capture: bool) -> Result<Stepped> {
        let actions = Action::of_frame(col, id, frame);
        let state = if self.mode == SyncMode::DjSync && self.has_write && actions.iter().any(|a| matches!(a, Action::Write { .. })) { Some(baseline.serialize().context("saving the baseline's state")?) } else { None };
        let bulk: Vec<&[u8]> = state.iter().map(Vec::as_slice).collect();
        let (v, mut bytes) = self.call("step", serde_json::json!({ "actions": actions, "capture": capture }), &bulk, &format!("while running frame {frame}"))?;
        let frame = match (&v["frame"], bytes.pop()) {
            (serde_json::Value::Object(size), Some(rgb)) => Some(Frame { width: size["width"].as_u64().unwrap_or(0) as u32, height: size["height"].as_u64().unwrap_or(0) as u32, rgb }),
            _ => None,
        };
        Ok(Stepped { frame, needs_sync: v["needs_sync"].as_bool().unwrap_or(false), tick_failed: v["tick_failed"].as_str().map(String::from) })
    }

    pub fn check(&mut self, baseline: &mut Core, power_on_frame: u64) -> Result<Verdict> {
        let state = baseline.serialize().context("saving the baseline's state")?;
        let what = if self.has_check { "while checking parity (dj_check_parity)" } else { "while comparing RAM" };
        let (v, _) = self.call("check", serde_json::json!({ "power_on_frame": power_on_frame }), &[&state], what)?;
        Ok(if let Some(w) = v["mismatch"].as_str() {
            Verdict::Mismatch(w.to_string(), serde_json::from_value(v["diffs"].clone()).unwrap_or_default())
        } else if let Some(w) = v["skip"].as_str() {
            Verdict::Skip(w.to_string())
        } else {
            Verdict::Match
        })
    }

    pub fn has_check(&self) -> bool {
        self.has_check
    }

    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    pub fn has_write(&self) -> bool {
        self.has_write
    }
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(w.compare[0].sync, SyncMode::DjSync);
        assert_eq!(w.compare[0].content, Some(Content::Rom(false)));
        assert_eq!(w.compare[0].options.get("x").map(String::as_str), Some("1"));
        assert_eq!(w.compare[1].sync, SyncMode::State);
        assert_eq!(w.compare[1].content, Some(Content::Path("other.nes".into())));
        assert_eq!(w.compare[2].content, None);
        assert!(toml::from_str::<W>("[[compare]]\nname = \"x\"\nsync = \"nope\"\n").is_err());
    }
}
