//! A minimal headless libretro frontend: loads a core, feeds it input, reads memory,
//! saves and loads states, captures video frames, and talks to a decomp-dj coverage connector.
//!
//! libretro cores keep global state and call back into plain C functions with no context
//! pointer. Each `Core` therefore sits in a slot (the baseline, and one comparison core):
//! every slot has its own frontend state in a thread-local and its own copies of the
//! callbacks, and one core library may be loaded into each slot.

use anyhow::{anyhow, bail, Context, Result};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::format::{InputKey, DEVICE_JOYPAD};

mod breaks;
mod connector;
mod ffi;
mod host;
mod load;

pub use breaks::*;
pub use connector::*;
pub use ffi::*;
pub use host::*;
pub use load::library_path;

struct Api {
    deinit: unsafe extern "C" fn(),
    run: unsafe extern "C" fn(),
    reset: unsafe extern "C" fn(),
    serialize_size: unsafe extern "C" fn() -> usize,
    serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool,
    load_game: unsafe extern "C" fn(*const GameInfo) -> bool,
    unload_game: unsafe extern "C" fn(),
    get_memory_data: unsafe extern "C" fn(u32) -> *mut c_void,
    get_memory_size: unsafe extern "C" fn(u32) -> usize,
    set_controller_port_device: unsafe extern "C" fn(u32, u32),
    get_system_av_info: unsafe extern "C" fn(*mut SystemAvInfo),
}

pub struct Core {
    _lib: libloading::Library,
    /// Which slot this core is in (BASELINE or COMPARISON).
    pub slot: usize,
    api: Api,
    pub library_name: String,
    pub library_version: String,
    connector: Option<Connector>,
    game_loaded: bool,
    /// The save state the core is in right now, when it is known: the one last saved or
    /// loaded, while nothing has run or been written since. Saving again then gives these
    /// bytes back without asking the core. Besides the time saved, a core may have to run on
    /// a little to reach a point where it can save (mupen64plus-next runs to its next
    /// interrupt when it is not at a frame's end), so a second save of the same frame would
    /// move its memory, and the frame's state hash with it.
    state_now: Option<Vec<u8>>,
    /// The device last set on each port: setting the same one again changes nothing.
    port_devices: BTreeMap<u32, u32>,
}

impl Core {
    /// Effective value of every option the core declared.
    pub fn effective_options(&self) -> BTreeMap<String, String> {
        with_host(self.slot, |h| {
            h.option_defs
                .iter()
                .map(|(k, (def, _))| (k.clone(), h.option_overrides.get(k).cloned().unwrap_or_else(|| def.clone())))
                .collect()
        })
        .unwrap_or_default()
    }

    pub fn load_game(&mut self, rom: &Path) -> Result<()> {
        let _call = CallGuard::enter(self.slot, 4);
        let data = std::fs::read(rom).with_context(|| format!("reading {}", rom.display()))?;
        let path = CString::new(rom.to_string_lossy().as_bytes())?;
        let info = GameInfo { path: path.as_ptr(), data: data.as_ptr() as *const c_void, size: data.len(), meta: std::ptr::null() };
        let ok = unsafe { (self.api.load_game)(&info) };
        if !ok {
            bail!("core refused to load {}{}", rom.display(), self.refused_note());
        }
        self.game_loaded = true;
        self.after_load()
    }

    /// ": it asked for ...", when the core asked for a GPU context it didn't get.
    fn refused_note(&self) -> String {
        with_host(self.slot, |h| h.hw_refused.clone()).flatten().map(|r| format!(": it asked for {r}, which isn't available")).unwrap_or_default()
    }

    /// After the game is loaded: reads the core's timing and geometry, and gives a core that
    /// draws with OpenGL its context.
    fn after_load(&mut self) -> Result<()> {
        let mut av = SystemAvInfo::default();
        unsafe { (self.api.get_system_av_info)(&mut av) };
        with_host(self.slot, |h| h.av_info = av);
        let Some((req, reset)) = with_host(self.slot, |h| h.hw.as_ref().map(|hw| (hw.req, hw.reset))).flatten() else { return Ok(()) };
        let g = av.geometry;
        let ctx = crate::gl::GlContext::new(req, g.max_width.max(g.base_width), g.max_height.max(g.base_height))
            .map_err(|e| anyhow!("the core draws with {}, and that failed: {e:#}", req.describe()))?;
        with_host(self.slot, |h| {
            if let Some(hw) = &mut h.hw {
                hw.ctx = Some(ctx);
            }
        });
        // The context is current: the core sets up its GL objects.
        if let Some(reset) = reset {
            unsafe { reset() };
        }
        self.gl_leave();
        Ok(())
    }

    /// Makes the core's GL context current (if it has one) for a call into the core.
    fn gl_enter(&self) {
        let r = with_host(self.slot, |h| {
            let g = h.av_info.geometry;
            h.hw.as_mut().and_then(|hw| hw.ctx.as_mut()).map(|c| c.enter(g.max_width, g.max_height))
        });
        if let Some(Some(Err(e))) = r {
            crate::console::warn("dj", &format!("{e:#}"));
        }
    }

    fn gl_leave(&self) {
        with_host(self.slot, |h| {
            if let Some(c) = h.hw.as_ref().and_then(|hw| hw.ctx.as_ref()) {
                c.leave();
            }
        });
    }

    /// The core draws with OpenGL: the context's description, for people.
    pub fn gpu(&self) -> Option<String> {
        with_host(self.slot, |h| h.hw.as_ref().and_then(|hw| hw.ctx.as_ref().map(|c| format!("{} ({})", hw.req.describe(), c.version)))).flatten()
    }

    /// Starts the core with no content (for cores that need none).
    pub fn load_no_game(&mut self) -> Result<()> {
        let ok = unsafe { (self.api.load_game)(std::ptr::null()) };
        if !ok {
            bail!("core refused to start without content{}", self.refused_note());
        }
        self.game_loaded = true;
        self.after_load()
    }

    /// A function or symbol of the core: through its proc address callback first
    /// (where connectors put their `dj_*` functions), then the library's exports.
    pub fn proc(&self, name: &str) -> *mut c_void {
        let Ok(c) = CString::new(name) else { return std::ptr::null_mut() };
        if let Some(gpa) = with_host(self.slot, |h| h.proc_iface).flatten() {
            if let Some(f) = unsafe { gpa(c.as_ptr()) } {
                return f as *mut c_void;
            }
        }
        unsafe { self._lib.get::<*mut c_void>(c.as_bytes_with_nul()).map(|s| *s).unwrap_or(std::ptr::null_mut()) }
    }

    /// Frames per second the core reports, and its preferred display aspect ratio (0 if unset).
    pub fn timing(&self) -> (f64, f32) {
        with_host(self.slot, |h| (h.av_info.timing.fps, h.av_info.geometry.aspect_ratio)).unwrap_or((0.0, 0.0))
    }

    pub fn has_connector(&self) -> bool {
        self.connector.is_some()
    }

    /// Which build of the core this is, for `[core] build`: the connector's `dj_build_id`
    /// (the upstream commit and the connector patch, the same on every platform), else the
    /// core's own name and version.
    pub fn build_id(&self) -> String {
        let f = self.proc("dj_build_id");
        if !f.is_null() {
            let f: unsafe extern "C" fn() -> *const c_char = unsafe { std::mem::transmute(f) };
            let p = unsafe { f() };
            if !p.is_null() {
                let id = unsafe { CStr::from_ptr(p) }.to_string_lossy().trim().to_string();
                if !id.is_empty() {
                    return id;
                }
            }
        }
        format!("{} {}", self.library_name, self.library_version)
    }

    pub fn run_frame(&mut self, capture: bool) -> Option<Frame> {
        let run = self.api.run;
        self.run_frame_with(capture, || unsafe { run() }).0
    }

    /// Runs one frame through `tick`, which calls `retro_run`, capturing the picture.
    fn run_frame_with<T>(&mut self, capture: bool, tick: impl FnOnce() -> T) -> (Option<Frame>, T) {
        let _call = CallGuard::enter(self.slot, 1);
        self.state_now = None;
        // A new frame: the cap counts from nothing, and what it switched off is back on.
        let rearm = BREAKS.with(|b| {
            let b = &mut b.borrow_mut()[self.slot];
            b.hits.clear();
            !std::mem::take(&mut b.capped).is_empty()
        });
        if rearm {
            self.arm_breakpoints();
        }
        with_host(self.slot, |h| {
            h.capture_next_frame = capture;
            if capture {
                h.last_frame = None;
            }
        });
        if self.slot == BASELINE {
            tell_hook(Stop::FrameStart);
        }
        self.gl_enter();
        let out = tick();
        self.gl_leave();
        // A write whose instruction didn't run this frame is dropped: it does not wait for a
        // later frame (a save state taken meanwhile wouldn't know of it).
        let left = BREAKS.with(|b| std::mem::take(&mut b.borrow_mut()[self.slot].writes));
        if !left.is_empty() {
            for w in &left {
                crate::console::count_write_not_made();
                crate::console::warn("dj", &format!("{}: its instruction didn't run in this frame, so the write was not made", w.label));
            }
            self.arm_breakpoints();
        }
        let frame = if capture {
            with_host(self.slot, |h| {
                h.capture_next_frame = false;
                h.last_frame.take()
            })
            .flatten()
        } else {
            None
        };
        (frame, out)
    }

    pub fn reset(&mut self) {
        let _call = CallGuard::enter(self.slot, 5);
        self.state_now = None;
        self.gl_enter();
        unsafe { (self.api.reset)() }
        self.gl_leave();
    }

    pub fn set_input(&mut self, key: InputKey, value: i16) {
        with_host(self.slot, |h| {
            if value == 0 {
                h.inputs.remove(&key);
            } else {
                h.inputs.insert(key, value);
            }
        });
    }

    pub fn clear_inputs(&mut self) {
        with_host(self.slot, |h| h.inputs.clear());
    }

    pub fn set_port_device(&mut self, port: u32, device: u32) {
        // (A port nothing was set on yet has libretro's default, the joypad.)
        if self.port_devices.insert(port, device).unwrap_or(crate::format::DEVICE_JOYPAD) != device {
            self.state_now = None;
        }
        unsafe { (self.api.set_controller_port_device)(port, device) }
    }

    pub fn serialize(&mut self) -> Result<Vec<u8>> {
        if let Some(s) = &self.state_now {
            return Ok(s.clone());
        }
        let _call = CallGuard::enter(self.slot, 2);
        self.gl_enter();
        self.breaks_busy(true);
        let r = self.serialize_inner();
        self.breaks_busy(false);
        self.gl_leave();
        if let Ok(s) = &r {
            self.state_now = Some(s.clone());
        }
        r
    }

    /// While a state is saved or loaded, breakpoints that fire are ignored.
    fn breaks_busy(&self, busy: bool) {
        BREAKS.with(|b| b.borrow_mut()[self.slot].busy = busy);
    }

    /// A comparison core's breakpoints in this core: (region index, offset). The frame hook
    /// is told when one is reached (`Stop::Break`).
    pub fn set_core_breakpoints(&mut self, list: &[(usize, u64)]) {
        BREAKS.with(|b| b.borrow_mut()[self.slot].core_breaks = list.iter().map(|(r, o)| (*r as i32, *o as i64)).collect());
        self.arm_breakpoints();
    }

    /// Arms a `[WRITE]` line that names an instruction (`at_region`, `at_offset`: a connector
    /// region and an offset): the bytes are written to `region` at `address` the first time
    /// that instruction is about to run in the next frame. If it doesn't run in that frame,
    /// the write is dropped with a warning. `label` names the line in messages.
    pub fn arm_write(&mut self, at_region: &str, at_offset: u64, region: &str, address: u64, bytes: &[u8], label: &str) -> Result<()> {
        if self.connector.is_none() {
            bail!("{label}: a write at an instruction needs a core with a dj connector, and this core has none");
        }
        let i = self.coverage_region_index(at_region).ok_or_else(|| anyhow!("{label}: this core's connector has no region {at_region}"))?;
        if region != "BUS" && region_memory_id(region).is_none() {
            bail!("{label}: unknown region {region}");
        }
        BREAKS.with(|b| b.borrow_mut()[self.slot].writes.push(ArmedWrite { at: (i as i32, at_offset as i64), region: region.to_string(), address, bytes: bytes.to_vec(), label: label.to_string() }));
        self.arm_breakpoints();
        Ok(())
    }

    /// A memory region's address and size, without borrowing the core mutably: for the
    /// handle a comparison core reads the baseline through while the baseline is mid-frame.
    pub fn memory_ptr(&self, id: u32) -> Option<(*const u8, usize)> {
        unsafe {
            let ptr = (self.api.get_memory_data)(id) as *const u8;
            let size = (self.api.get_memory_size)(id);
            (!ptr.is_null() && size > 0).then_some((ptr, size))
        }
    }

    /// Runs `f`, a call into this core that isn't one of libretro's (a comparison core's
    /// event function), with its GL context current and the call named for a crash note.
    pub fn in_core<T>(&mut self, call: usize, f: impl FnOnce() -> T) -> T {
        self.state_now = None;
        let _call = CallGuard::enter(self.slot, call);
        self.gl_enter();
        let out = f();
        self.gl_leave();
        out
    }

    /// Whether the next picture the core hands over is kept (`capture_take`). With the
    /// video hint on, the core is also told whether one is wanted.
    pub fn capture_begin(&mut self, capture: bool) {
        with_host(self.slot, |h| {
            h.capture_next_frame = capture;
            if capture {
                h.last_frame = None;
            }
        });
    }

    /// The picture kept since `capture_begin`, if the core handed one over.
    pub fn capture_take(&mut self) -> Option<Frame> {
        with_host(self.slot, |h| {
            h.capture_next_frame = false;
            h.last_frame.take()
        })
        .flatten()
    }

    /// Tells the core, through `GET_AUDIO_VIDEO_ENABLE`, whether each picture is wanted.
    pub fn set_video_hint(&mut self, on: bool) {
        with_host(self.slot, |h| h.video_hint = on);
    }

    /// Gives the connector dj's breakpoints: the `[tick]` address, a comparison core's, and
    /// the instructions this frame's writes wait for.
    fn arm_breakpoints(&mut self) {
        let Some(c) = &self.connector else { return };
        let list: Vec<DjBreakC> = BREAKS.with(|b| {
            let b = &b.borrow()[self.slot];
            let mut places: Vec<(i32, i64)> = b.tick.iter().copied().chain(b.core_breaks.iter().copied()).chain(b.writes.iter().map(|w| w.at)).collect();
            places.sort();
            places.dedup();
            places.into_iter().map(|(region, offset)| DjBreakC { region, offset }).collect()
        });
        let cb: BreakFn = match self.slot {
            BASELINE => break_cb::<0>,
            _ => break_cb::<1>,
        };
        unsafe { (c.set_breakpoints)(list.as_ptr(), list.len(), if list.is_empty() { None } else { Some(cb) }, std::ptr::null_mut()) }
    }

    /// The CPU's registers as the connector hands them out (`dj_read_registers`,
    /// docs/connectors.md has the layout per instruction set). None: no connector, or it
    /// can't say.
    pub fn registers(&self) -> Option<Vec<u8>> {
        let f = self.connector.as_ref()?.registers?;
        let mut buf = vec![0u8; 512];
        let n = unsafe { f(buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n == 0 || n > buf.len() {
            return None;
        }
        buf.truncate(n);
        Some(buf)
    }

    fn serialize_inner(&mut self) -> Result<Vec<u8>> {
        unsafe {
            let size = (self.api.serialize_size)();
            if size == 0 {
                bail!("core does not support save states");
            }
            let mut buf = vec![0u8; size];
            if !(self.api.serialize)(buf.as_mut_ptr() as *mut c_void, size) {
                bail!("core failed to save state");
            }
            Ok(buf)
        }
    }

    pub fn unserialize(&mut self, state: &[u8]) -> Result<()> {
        let _call = CallGuard::enter(self.slot, 3);
        self.gl_enter();
        self.breaks_busy(true);
        let ok = unsafe { (self.api.unserialize)(state.as_ptr() as *const c_void, state.len()) };
        self.breaks_busy(false);
        self.gl_leave();
        if !ok {
            self.state_now = None;
            bail!("core failed to load state");
        }
        match &mut self.state_now {
            Some(s) if s.len() == state.len() => s.copy_from_slice(state),
            now => *now = Some(state.to_vec()),
        }
        Ok(())
    }

    /// A memory region to read. (`memory` is for writing: after it, the core's state is no
    /// longer the one last saved or loaded.)
    pub fn memory_view(&self, id: u32) -> Option<&[u8]> {
        let (ptr, size) = self.memory_ptr(id)?;
        Some(unsafe { std::slice::from_raw_parts(ptr, size) })
    }

    /// A memory region to write to (to read, `memory_view`).
    pub fn memory(&mut self, id: u32) -> Option<&mut [u8]> {
        self.state_now = None;
        unsafe {
            let ptr = (self.api.get_memory_data)(id) as *mut u8;
            let size = (self.api.get_memory_size)(id);
            if ptr.is_null() || size == 0 {
                None
            } else {
                Some(std::slice::from_raw_parts_mut(ptr, size))
            }
        }
    }

    /// Resolves an emulated bus address through the core's memory maps.
    /// To write through (to read, `bus_read`).
    pub fn bus_byte(&mut self, addr: usize) -> Option<*mut u8> {
        self.state_now = None;
        bus_ptr(self.slot, addr)
    }

    /// The byte at an emulated bus address.
    pub fn bus_read(&self, addr: usize) -> Option<u8> {
        bus_ptr(self.slot, addr).map(|p| unsafe { *p })
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // As RetroArch: the GL context is destroyed (context_destroy) before the game unloads.
        let destroy = with_host(self.slot, |h| h.hw.as_ref().filter(|hw| hw.ctx.is_some()).map(|hw| hw.destroy)).flatten();
        self.gl_enter();
        unsafe {
            if let Some(Some(d)) = destroy {
                d();
            }
            if self.game_loaded {
                (self.api.unload_game)();
            }
            (self.api.deinit)();
        }
        self.gl_leave();
        // Drop the context outside the borrow (dropping it makes it current again).
        let ctx = with_host(self.slot, |h| h.hw.as_mut().and_then(|hw| hw.ctx.take())).flatten();
        drop(ctx);
        HOST.with(|h| h.borrow_mut()[self.slot] = None);
        BREAKS.with(|b| b.borrow_mut()[self.slot] = Breaks::new());
        CORE_LOADED[self.slot].store(false, Ordering::SeqCst);
        CORE_RUN[self.slot].store(0, Ordering::SeqCst);
    }
}

/// Names the libretro call in progress for a crash note (`crash::enter`, or
/// `crash::enter_compare` for the comparison core's slot), until dropped.
struct CallGuard(usize);

impl CallGuard {
    fn enter(slot: usize, call: usize) -> CallGuard {
        if slot == COMPARISON {
            crate::crash::enter_compare(call);
        } else {
            crate::crash::enter(call);
        }
        CallGuard(slot)
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if self.0 == COMPARISON {
            crate::crash::enter_compare(0);
        } else {
            crate::crash::enter(0);
        }
    }
}

pub fn write_png(frame: &Frame, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&frame.rgb)?;
    Ok(())
}

pub fn region_memory_id(name: &str) -> Option<u32> {
    Some(match name {
        "RAM" => MEMORY_SYSTEM_RAM,
        "SRAM" => MEMORY_SAVE_RAM,
        "VRAM" => MEMORY_VIDEO_RAM,
        "RTC" => MEMORY_RTC,
        _ => return None,
    })
}
