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
use crate::format::Command;
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
    /// The cores disagree after this frame, with the reason.
    Mismatch(String),
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
        let cfg = col.config.compare.iter().find(|c| c.name == name).ok_or_else(|| anyhow!("no [[compare]] entry is named \"{name}\""))?;
        let path = cfg.path.as_ref().ok_or_else(|| anyhow!("[[compare]] \"{name}\" has no path: set `path` (in config.toml, or in .local/local.toml for this machine)"))?;
        let path = crate::libretro::library_path(&if path.is_relative() { col.root.join(path) } else { path.clone() });
        let baseline_core = &crate::libretro::library_path(baseline_core);
        if !path.is_file() {
            bail!("the core library {} doesn't exist", path.display());
        }
        let build = sha256_file(&path)?;
        if !cfg.sha256.is_empty() && build != cfg.sha256 {
            bail!("core {} has sha256 {build}, but [[compare]] \"{name}\" is pinned to {}", path.display(), cfg.sha256);
        }
        let work = col.local_dir().join("compare").join(sanitize(name));
        std::fs::create_dir_all(&work)?;
        // A library is loaded once per process: the baseline's own file would share its globals.
        let same = std::fs::canonicalize(&path).ok().is_some_and(|p| std::fs::canonicalize(baseline_core).ok() == Some(p));
        let (load_path, copy) = if same {
            let file = path.file_name().ok_or_else(|| anyhow!("bad core path {}", path.display()))?.to_string_lossy().into_owned();
            let copy = work.join(format!("{}-{}", std::process::id(), file));
            std::fs::copy(&path, &copy).with_context(|| format!("copying {} to load it a second time", path.display()))?;
            (copy.clone(), Some(copy))
        } else {
            (path.clone(), None)
        };
        let loaded = (|| -> Result<Comparison> {
            let mut core = Core::load_in(COMPARISON, &load_path, &work, &cfg.options)?;
            match &cfg.content {
                None | Some(Content::Rom(true)) => {
                    let rom = rom.ok_or_else(|| anyhow!("no ROM is set for this project"))?;
                    core.load_game(rom)?;
                }
                Some(Content::Rom(false)) => core.load_no_game()?,
                Some(Content::Path(p)) => core.load_game(&if p.is_relative() { col.root.join(p) } else { p.clone() })?,
            }
            let sync_ptr = core.proc("dj_sync");
            let write_ptr = core.proc("dj_write");
            let sync_fn: Option<SyncFn> = (!sync_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, SyncFn>(sync_ptr) });
            let write_fn: Option<WriteFn> = (!write_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, WriteFn>(write_ptr) });
            let check_ptr = core.proc("dj_check_parity");
            let run_ptr = core.proc("dj_run");
            let check_fn: Option<CheckFn> = (!check_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, CheckFn>(check_ptr) });
            let run_fn: Option<RunFn> = (!run_ptr.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, RunFn>(run_ptr) });
            match cfg.sync {
                SyncMode::DjSync if sync_fn.is_none() => bail!("the core has no dj_sync, which [[compare]] \"{name}\" asks for (sync = \"dj_sync\", the default). For the same emulator as the baseline, sync = \"state\" syncs by save state instead"),
                SyncMode::State => {
                    core.serialize().context("sync = \"state\" needs save states, and this core can't save one")?;
                }
                _ => {}
            }
            Ok(Comparison { name: name.to_string(), core, mode: cfg.sync, sync_fn, write_fn, check_fn, run_fn, build: build.clone(), copy: None })
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
            restore_inputs_on(&mut self.core, col, id, frame)?;
        }
        Ok(out)
    }

    /// Sets devices and releases inputs at frame 0 of `id`, as the baseline does when
    /// playback carries straight on into a child segment.
    pub fn enter(&mut self, col: &Collection, id: &str) -> Result<()> {
        restore_inputs_on(&mut self.core, col, id, 0)
    }

    /// Applies frame `frame`'s lines of `id` and runs that frame. `baseline` is handed to
    /// `dj_write` for `[WRITE]` lines.
    pub fn step(&mut self, baseline: &Core, col: &Collection, id: &str, frame: u64, capture: bool) -> Result<Stepped> {
        let r = &col.replays[id];
        let mut impulses = vec![];
        let mut needs_sync = false;
        let first = r.lines.partition_point(|l| l.frame < frame);
        for line in r.lines[first..].iter().take_while(|l| l.frame == frame) {
            match &line.command {
                Command::Input { key, value, impulse } => {
                    self.core.set_input(*key, *value);
                    if *impulse {
                        impulses.push(*key);
                    }
                }
                Command::Device { port, device } => self.core.set_port_device(*port, *device),
                Command::Write { region, address, bytes } => match (self.mode, self.write_fn) {
                    // The same emulator: the write goes straight into its memory.
                    (SyncMode::State, _) => write_memory(&mut self.core, region, *address, bytes)?,
                    (SyncMode::DjSync, Some(f)) => {
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
                Command::Event { .. } | Command::Screenshot => {}
                Command::Reset => self.core.reset(),
                Command::Rng { .. } => bail!("[RNG] is not supported yet"),
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
                DJ_PARITY_MISMATCH => Ok(Verdict::Mismatch(reason(&buf).unwrap_or_else(|| "the core reports a mismatch".into()))),
                DJ_PARITY_SKIP => Ok(Verdict::Skip(reason(&buf).unwrap_or_else(|| "not checked here".into()))),
                _ => bail!("dj_check_parity failed: {}", reason(&buf).unwrap_or_else(|| format!("error {r}"))),
            };
        }
        let base = baseline.memory(MEMORY_SYSTEM_RAM).map(|m| m.to_vec());
        let mine = self.core.memory(MEMORY_SYSTEM_RAM);
        Ok(match (base, mine) {
            (Some(a), Some(b)) if a.len() == b.len() => match a.iter().zip(b.iter()).position(|(x, y)| x != y) {
                None => Verdict::Match,
                Some(i) => Verdict::Mismatch(format!("system RAM differs at 0x{i:X}: 0x{:02X} here, 0x{:02X} in the baseline", b[i], a[i])),
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

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
