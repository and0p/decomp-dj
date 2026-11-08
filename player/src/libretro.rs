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
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::format::{InputKey, DEVICE_JOYPAD};

// ---- libretro ABI ----------------------------------------------------------

const ENV_EXPERIMENTAL: u32 = 0x10000;
const ENV_GET_CAN_DUPE: u32 = 3;
const ENV_GET_SYSTEM_DIRECTORY: u32 = 9;
const ENV_SET_PIXEL_FORMAT: u32 = 10;
const ENV_SET_INPUT_DESCRIPTORS: u32 = 11;
const ENV_GET_VARIABLE: u32 = 15;
const ENV_SET_VARIABLES: u32 = 16;
const ENV_GET_VARIABLE_UPDATE: u32 = 17;
const ENV_SET_SUPPORT_NO_GAME: u32 = 18;
const ENV_GET_SAVE_DIRECTORY: u32 = 31;
const ENV_SET_SYSTEM_AV_INFO: u32 = 32;
const ENV_SET_PROC_ADDRESS_CALLBACK: u32 = 33;
const ENV_SET_CONTROLLER_INFO: u32 = 35;
const ENV_SET_MEMORY_MAPS: u32 = 36 | ENV_EXPERIMENTAL;
const ENV_SET_GEOMETRY: u32 = 37;
const ENV_GET_LANGUAGE: u32 = 39;
const ENV_SET_SERIALIZATION_QUIRKS: u32 = 44;
const ENV_GET_AUDIO_VIDEO_ENABLE: u32 = 47 | ENV_EXPERIMENTAL;
const ENV_GET_FASTFORWARDING: u32 = 49 | ENV_EXPERIMENTAL;
const ENV_GET_CORE_OPTIONS_VERSION: u32 = 52;
const ENV_SET_CORE_OPTIONS_DISPLAY: u32 = 55;
const ENV_GET_INPUT_MAX_USERS: u32 = 61;
const ENV_SET_HW_RENDER: u32 = 14;
const ENV_GET_HW_RENDER_INTERFACE: u32 = 41 | ENV_EXPERIMENTAL;
const ENV_SET_HW_SHARED_CONTEXT: u32 = 44 | ENV_EXPERIMENTAL;
const ENV_GET_PREFERRED_HW_RENDER: u32 = 56;

const HW_CONTEXT_OPENGL: u32 = 1;
const HW_CONTEXT_OPENGL_CORE: u32 = 3;
/// `RETRO_HW_FRAME_BUFFER_VALID`: the frame is in the framebuffer, not in memory.
const HW_FRAME_BUFFER_VALID: usize = usize::MAX;

pub const MEMORY_SAVE_RAM: u32 = 0;
pub const MEMORY_RTC: u32 = 1;
pub const MEMORY_SYSTEM_RAM: u32 = 2;
pub const MEMORY_VIDEO_RAM: u32 = 3;

const DEVICE_ID_JOYPAD_MASK: u32 = 256;

#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}

#[repr(C)]
struct SystemInfo {
    library_name: *const c_char,
    library_version: *const c_char,
    valid_extensions: *const c_char,
    need_fullpath: bool,
    block_extract: bool,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct GameGeometry {
    base_width: u32,
    base_height: u32,
    max_width: u32,
    max_height: u32,
    aspect_ratio: f32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SystemTiming {
    fps: f64,
    sample_rate: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SystemAvInfo {
    geometry: GameGeometry,
    timing: SystemTiming,
}

#[repr(C)]
struct Variable {
    key: *const c_char,
    value: *const c_char,
}

type ProcAddress = Option<unsafe extern "C" fn()>;

#[repr(C)]
struct GetProcAddressInterface {
    get_proc_address: Option<unsafe extern "C" fn(*const c_char) -> ProcAddress>,
}

#[repr(C)]
struct MemoryDescriptor {
    flags: u64,
    ptr: *mut c_void,
    offset: usize,
    start: usize,
    select: usize,
    disconnect: usize,
    len: usize,
    addrspace: *const c_char,
}

#[repr(C)]
struct MemoryMap {
    descriptors: *const MemoryDescriptor,
    num_descriptors: u32,
}

#[repr(C)]
struct DjRegionC {
    name: *const c_char,
    flags: *mut u8,
    size: usize,
}

type HwResetFn = unsafe extern "C" fn();

/// `struct retro_hw_render_callback`.
#[repr(C)]
struct HwRenderCallback {
    context_type: u32,
    context_reset: Option<HwResetFn>,
    get_current_framebuffer: Option<unsafe extern "C" fn() -> usize>,
    get_proc_address: Option<unsafe extern "C" fn(*const c_char) -> ProcAddress>,
    depth: bool,
    stencil: bool,
    bottom_left_origin: bool,
    version_major: u32,
    version_minor: u32,
    cache_context: bool,
    context_destroy: Option<HwResetFn>,
    debug_context: bool,
}

type EnvFn = unsafe extern "C" fn(u32, *mut c_void) -> bool;
type VideoFn = unsafe extern "C" fn(*const c_void, u32, u32, usize);
type AudioFn = unsafe extern "C" fn(i16, i16);
type AudioBatchFn = unsafe extern "C" fn(*const i16, usize) -> usize;
type PollFn = unsafe extern "C" fn();
type StateFn = unsafe extern "C" fn(u32, u32, u32, u32) -> i16;

// ---- frontend state reached from callbacks ----------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb1555,
    Xrgb8888,
    Rgb565,
}

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// RGB, 3 bytes per pixel.
    pub rgb: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct BusRange {
    pub start: usize,
    pub select: usize,
    pub disconnect: usize,
    pub len: usize,
    pub ptr: usize,
    pub offset: usize,
}

/// A core that draws with OpenGL (`SET_HW_RENDER`).
struct Hw {
    req: crate::gl::Request,
    reset: Option<HwResetFn>,
    destroy: Option<HwResetFn>,
    bottom_left_origin: bool,
    /// Made once the game is loaded.
    ctx: Option<crate::gl::GlContext>,
}

struct Host {
    system_dir: CString,
    save_dir: CString,
    pixel_format: PixelFormat,
    /// Option key -> (default value, allowed values)
    option_defs: BTreeMap<String, (String, Vec<String>)>,
    option_overrides: BTreeMap<String, String>,
    option_cstrings: HashMap<String, CString>,
    options_dirty: bool,
    inputs: HashMap<InputKey, i16>,
    capture_next_frame: bool,
    last_frame: Option<Frame>,
    proc_iface: Option<unsafe extern "C" fn(*const c_char) -> ProcAddress>,
    bus: Vec<BusRange>,
    av_info: SystemAvInfo,
    hw: Option<Hw>,
    /// A GPU context the core asked for and was refused, and why.
    hw_refused: Option<String>,
}

/// How many cores may be loaded at once.
pub const SLOTS: usize = 2;
/// The slot of the project's own core.
pub const BASELINE: usize = 0;
/// The slot of a comparison core.
pub const COMPARISON: usize = 1;

thread_local! {
    static HOST: RefCell<[Option<Host>; SLOTS]> = const { RefCell::new([None, None]) };
}

static CORE_LOADED: [AtomicBool; SLOTS] = [AtomicBool::new(false), AtomicBool::new(false)];

fn with_host<R>(slot: usize, f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| h.borrow_mut()[slot].as_mut().map(f))
}

/// Parses a v0 core option string: "Description; default|other|other".
fn parse_option_values(v: &str) -> (String, Vec<String>) {
    let list = v.split_once("; ").map(|(_, l)| l).unwrap_or(v);
    let values: Vec<String> = list.split('|').map(|s| s.to_string()).collect();
    (values.first().cloned().unwrap_or_default(), values)
}

unsafe extern "C" fn env_cb<const S: usize>(cmd: u32, data: *mut c_void) -> bool {
    match cmd {
        ENV_GET_CAN_DUPE => {
            *(data as *mut bool) = true;
            true
        }
        ENV_GET_SYSTEM_DIRECTORY | ENV_GET_SAVE_DIRECTORY => with_host(S, |h| {
            let p = if cmd == ENV_GET_SYSTEM_DIRECTORY { h.system_dir.as_ptr() } else { h.save_dir.as_ptr() };
            *(data as *mut *const c_char) = p;
            true
        })
        .unwrap_or(false),
        ENV_SET_PIXEL_FORMAT => {
            let f = match *(data as *const u32) {
                0 => PixelFormat::Rgb1555,
                1 => PixelFormat::Xrgb8888,
                2 => PixelFormat::Rgb565,
                _ => return false,
            };
            with_host(S, |h| h.pixel_format = f);
            true
        }
        ENV_SET_VARIABLES => {
            let mut v = data as *const Variable;
            with_host(S, |h| {
                while !(*v).key.is_null() {
                    let key = CStr::from_ptr((*v).key).to_string_lossy().into_owned();
                    let val = CStr::from_ptr((*v).value).to_string_lossy().into_owned();
                    h.option_defs.insert(key, parse_option_values(&val));
                    v = v.add(1);
                }
            });
            true
        }
        ENV_GET_VARIABLE => {
            let var = &mut *(data as *mut Variable);
            if var.key.is_null() {
                return false;
            }
            let key = CStr::from_ptr(var.key).to_string_lossy().into_owned();
            with_host(S, |h| {
                let value = h.option_overrides.get(&key).cloned().or_else(|| h.option_defs.get(&key).map(|d| d.0.clone()));
                match value {
                    Some(v) => {
                        let c = h.option_cstrings.entry(key).or_insert_with(|| CString::new(v.clone()).unwrap());
                        if c.to_str().ok() != Some(v.as_str()) {
                            *c = CString::new(v).unwrap();
                        }
                        var.value = c.as_ptr();
                        true
                    }
                    None => {
                        var.value = std::ptr::null();
                        false
                    }
                }
            })
            .unwrap_or(false)
        }
        ENV_GET_VARIABLE_UPDATE => {
            let dirty = with_host(S, |h| std::mem::replace(&mut h.options_dirty, false)).unwrap_or(false);
            *(data as *mut bool) = dirty;
            true
        }
        ENV_GET_CORE_OPTIONS_VERSION => {
            // Version 0 makes cores fall back to SET_VARIABLES, which is all we need.
            *(data as *mut u32) = 0;
            true
        }
        ENV_SET_PROC_ADDRESS_CALLBACK => {
            let iface = &*(data as *const GetProcAddressInterface);
            with_host(S, |h| h.proc_iface = iface.get_proc_address);
            true
        }
        ENV_SET_MEMORY_MAPS => {
            let map = &*(data as *const MemoryMap);
            let mut ranges = vec![];
            for i in 0..map.num_descriptors as usize {
                let d = &*map.descriptors.add(i);
                ranges.push(BusRange {
                    start: d.start,
                    select: d.select,
                    disconnect: d.disconnect,
                    len: d.len,
                    ptr: d.ptr as usize,
                    offset: d.offset,
                });
            }
            with_host(S, |h| h.bus = ranges);
            true
        }
        ENV_SET_SYSTEM_AV_INFO => {
            let av = *(data as *const SystemAvInfo);
            with_host(S, |h| h.av_info = av);
            true
        }
        ENV_SET_GEOMETRY => {
            let g = *(data as *const GameGeometry);
            with_host(S, |h| h.av_info.geometry = g);
            true
        }
        ENV_GET_LANGUAGE => {
            *(data as *mut u32) = 0;
            true
        }
        ENV_GET_AUDIO_VIDEO_ENABLE => {
            // bit 0: video, bit 1: audio. Keep video on so frames can be captured.
            *(data as *mut i32) = 1;
            true
        }
        ENV_GET_FASTFORWARDING => {
            *(data as *mut bool) = false;
            true
        }
        ENV_GET_INPUT_MAX_USERS => {
            *(data as *mut u32) = 8;
            true
        }
        ENV_SET_HW_RENDER => {
            let cb = &mut *(data as *mut HwRenderCallback);
            let req = crate::gl::Request {
                core_profile: cb.context_type == HW_CONTEXT_OPENGL_CORE,
                major: cb.version_major,
                minor: cb.version_minor,
                depth: cb.depth,
                stencil: cb.stencil,
                debug: cb.debug_context,
            };
            let refused = match cb.context_type {
                HW_CONTEXT_OPENGL | HW_CONTEXT_OPENGL_CORE => crate::gl::unsupported(&req),
                t => Some(format!("{} (only OpenGL is supported so far)", hw_context_name(t))),
            };
            if let Some(why) = refused {
                with_host(S, |h| h.hw_refused = Some(why));
                return false;
            }
            cb.get_current_framebuffer = Some(hw_framebuffer_cb::<S>);
            cb.get_proc_address = Some(hw_proc_cb::<S>);
            let hw = Hw { req, reset: cb.context_reset, destroy: cb.context_destroy, bottom_left_origin: cb.bottom_left_origin, ctx: None };
            with_host(S, |h| {
                h.hw = Some(hw);
                h.hw_refused = None;
            });
            true
        }
        ENV_GET_PREFERRED_HW_RENDER => {
            if crate::gl::unsupported(&crate::gl::Request { core_profile: false, major: 0, minor: 0, depth: false, stencil: false, debug: false }).is_some() {
                return false;
            }
            // macOS has no compatibility profile past 2.1.
            *(data as *mut u32) = if cfg!(target_os = "macos") { HW_CONTEXT_OPENGL_CORE } else { HW_CONTEXT_OPENGL };
            true
        }
        // Every core gets a context of its own anyway.
        ENV_SET_HW_SHARED_CONTEXT => true,
        ENV_GET_HW_RENDER_INTERFACE => false,
        ENV_SET_INPUT_DESCRIPTORS | ENV_SET_CONTROLLER_INFO | ENV_SET_SUPPORT_NO_GAME | ENV_SET_CORE_OPTIONS_DISPLAY
        | ENV_SET_SERIALIZATION_QUIRKS => true,
        _ => false,
    }
}

fn hw_context_name(t: u32) -> String {
    match t {
        1 => "OpenGL".into(),
        2 => "OpenGL ES 2".into(),
        3 => "OpenGL core".into(),
        4 => "OpenGL ES 3".into(),
        5 => "OpenGL ES".into(),
        6 => "Vulkan".into(),
        7 => "Direct3D 11".into(),
        8 => "Direct3D 10".into(),
        9 => "Direct3D 12".into(),
        10 => "Direct3D 9".into(),
        t => format!("GPU context type {t}"),
    }
}

unsafe extern "C" fn hw_framebuffer_cb<const S: usize>() -> usize {
    with_host(S, |h| h.hw.as_ref().and_then(|hw| hw.ctx.as_ref()).map_or(0, |c| c.framebuffer() as usize)).unwrap_or(0)
}

unsafe extern "C" fn hw_proc_cb<const S: usize>(name: *const c_char) -> ProcAddress {
    if name.is_null() {
        return None;
    }
    let name = CStr::from_ptr(name).to_string_lossy();
    let p = with_host(S, |h| h.hw.as_ref().and_then(|hw| hw.ctx.as_ref()).map_or(std::ptr::null(), |c| c.proc(&name))).unwrap_or(std::ptr::null());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute::<*const c_void, unsafe extern "C" fn()>(p))
    }
}

/// A colour channel of `max + 1` levels (the bits of `v` under `max`), widened to 0..=255.
fn scale(v: u16, max: u16) -> u8 {
    ((v & max) as u32 * 255 / max as u32) as u8
}

unsafe extern "C" fn video_cb<const S: usize>(data: *const c_void, width: u32, height: u32, pitch: usize) {
    with_host(S, |h| {
        if !h.capture_next_frame || data.is_null() {
            return;
        }
        h.capture_next_frame = false;
        if data as usize == HW_FRAME_BUFFER_VALID {
            // Drawn on the GPU: read it back (the context is current while the core runs).
            if let Some((ctx, bl)) = h.hw.as_ref().and_then(|hw| hw.ctx.as_ref().map(|c| (c, hw.bottom_left_origin))) {
                let rgb = ctx.read_rgb(width, height, bl);
                let (w, hgt) = (width.min(ctx.width), height.min(ctx.height));
                h.last_frame = Some(Frame { width: w, height: hgt, rgb });
            }
            return;
        }
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height as usize {
            let row = (data as *const u8).add(y * pitch);
            for x in 0..width as usize {
                let (r, g, b) = match h.pixel_format {
                    PixelFormat::Xrgb8888 => {
                        let p = *(row.add(x * 4) as *const u32);
                        ((p >> 16) as u8, (p >> 8) as u8, p as u8)
                    }
                    PixelFormat::Rgb565 => {
                        let p = *(row.add(x * 2) as *const u16);
                        (scale(p >> 11, 0x1f), scale(p >> 5, 0x3f), scale(p, 0x1f))
                    }
                    PixelFormat::Rgb1555 => {
                        let p = *(row.add(x * 2) as *const u16);
                        (scale(p >> 10, 0x1f), scale(p >> 5, 0x1f), scale(p, 0x1f))
                    }
                };
                rgb.extend_from_slice(&[r, g, b]);
            }
        }
        h.last_frame = Some(Frame { width, height, rgb });
    });
}

unsafe extern "C" fn audio_cb(_l: i16, _r: i16) {}
unsafe extern "C" fn audio_batch_cb(_data: *const i16, frames: usize) -> usize {
    frames
}
unsafe extern "C" fn input_poll_cb() {}

unsafe extern "C" fn input_state_cb<const S: usize>(port: u32, device: u32, index: u32, id: u32) -> i16 {
    let base = device & 0xff;
    with_host(S, |h| {
        if base == DEVICE_JOYPAD && id == DEVICE_ID_JOYPAD_MASK {
            let mut mask = 0i16;
            for b in 0..16u32 {
                if h.inputs.get(&InputKey { port, device: base, index: 0, id: b }).copied().unwrap_or(0) != 0 {
                    mask |= 1 << b;
                }
            }
            return mask;
        }
        h.inputs.get(&InputKey { port, device: base, index, id }).copied().unwrap_or(0)
    })
    .unwrap_or(0)
}

// ---- the core --------------------------------------------------------------

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

pub struct Connector {
    region_count: unsafe extern "C" fn() -> usize,
    get_region: unsafe extern "C" fn(usize, *mut DjRegionC) -> bool,
    clear: unsafe extern "C" fn(),
    set_tick_exec: unsafe extern "C" fn(i32, i64),
    get_tick_count: unsafe extern "C" fn() -> u64,
    /// The optional trace part of the API (dj_trace_*).
    trace: Option<TraceFns>,
}

struct TraceFns {
    start: unsafe extern "C" fn(usize),
    stop: unsafe extern "C" fn(),
    read: unsafe extern "C" fn(*mut *const TraceEvent, *mut u64) -> usize,
}

pub const TRACE_EXEC: u8 = 1;
pub const TRACE_READ: u8 = 2;
pub const TRACE_WRITE: u8 = 3;

/// One traced instruction or memory access (`struct dj_trace_event`, docs/connectors.md).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TraceEvent {
    pub cycle: u64,
    /// Offset into `region`, or -1.
    pub offset: i64,
    /// CPU address.
    pub addr: u16,
    pub kind: u8,
    /// Region index as in the coverage regions, or -1.
    pub region: i8,
    pub value: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub operand_count: u8,
    pub operands: [u8; 3],
    pub reserved: [u8; 4],
}

pub struct CoverageRegion {
    pub name: String,
    pub flags: Vec<u8>,
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
}

impl Core {
    /// Loads the project's core, into the baseline slot.
    pub fn load(core_path: &Path, work_dir: &Path, options: &BTreeMap<String, String>) -> Result<Core> {
        Self::load_in(BASELINE, core_path, work_dir, options)
    }

    /// Loads a core into `slot`. Two slots must not hold the same library file: a library
    /// is only loaded once per process, so the second would share the first one's globals.
    pub fn load_in(slot: usize, core_path: &Path, work_dir: &Path, options: &BTreeMap<String, String>) -> Result<Core> {
        if slot >= SLOTS {
            bail!("no core slot {slot}");
        }
        if CORE_LOADED[slot].swap(true, Ordering::SeqCst) {
            bail!("a libretro core is already loaded in slot {slot}");
        }
        let r = Self::load_unchecked(slot, core_path, work_dir, options);
        if r.is_err() {
            HOST.with(|h| h.borrow_mut()[slot] = None);
            CORE_LOADED[slot].store(false, Ordering::SeqCst);
        }
        r
    }

    fn load_unchecked(slot: usize, core_path: &Path, work_dir: &Path, options: &BTreeMap<String, String>) -> Result<Core> {
        let system_dir = work_dir.join("system");
        let save_dir = work_dir.join("saves");
        std::fs::create_dir_all(&system_dir)?;
        std::fs::create_dir_all(&save_dir)?;
        HOST.with(|h| {
            h.borrow_mut()[slot] = Some(Host {
                system_dir: CString::new(system_dir.to_string_lossy().as_bytes()).unwrap(),
                save_dir: CString::new(save_dir.to_string_lossy().as_bytes()).unwrap(),
                pixel_format: PixelFormat::Rgb1555,
                option_defs: BTreeMap::new(),
                option_overrides: options.clone(),
                option_cstrings: HashMap::new(),
                options_dirty: false,
                inputs: HashMap::new(),
                capture_next_frame: false,
                last_frame: None,
                proc_iface: None,
                bus: vec![],
                av_info: SystemAvInfo::default(),
                hw: None,
                hw_refused: None,
            })
        });

        unsafe {
            let lib = libloading::Library::new(core_path).with_context(|| format!("loading core {}", core_path.display()))?;
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).with_context(|| format!("core is missing {}", $name))?
                };
            }
            let set_environment: unsafe extern "C" fn(EnvFn) = sym!("retro_set_environment");
            let set_video: unsafe extern "C" fn(VideoFn) = sym!("retro_set_video_refresh");
            let set_audio: unsafe extern "C" fn(AudioFn) = sym!("retro_set_audio_sample");
            let set_audio_batch: unsafe extern "C" fn(AudioBatchFn) = sym!("retro_set_audio_sample_batch");
            let set_poll: unsafe extern "C" fn(PollFn) = sym!("retro_set_input_poll");
            let set_state: unsafe extern "C" fn(StateFn) = sym!("retro_set_input_state");
            let init: unsafe extern "C" fn() = sym!("retro_init");
            let get_system_info: unsafe extern "C" fn(*mut SystemInfo) = sym!("retro_get_system_info");
            let api = Api {
                deinit: sym!("retro_deinit"),
                run: sym!("retro_run"),
                reset: sym!("retro_reset"),
                serialize_size: sym!("retro_serialize_size"),
                serialize: sym!("retro_serialize"),
                unserialize: sym!("retro_unserialize"),
                load_game: sym!("retro_load_game"),
                unload_game: sym!("retro_unload_game"),
                get_memory_data: sym!("retro_get_memory_data"),
                get_memory_size: sym!("retro_get_memory_size"),
                set_controller_port_device: sym!("retro_set_controller_port_device"),
                get_system_av_info: sym!("retro_get_system_av_info"),
            };

            let mut info: SystemInfo = std::mem::zeroed();
            get_system_info(&mut info);
            let cstr = |p: *const c_char| if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
            let library_name = cstr(info.library_name);
            let library_version = cstr(info.library_version);

            // Same order as RetroArch: some cores (Mesen) only build their state in retro_init.
            // Each slot has its own copies of the callbacks, which reach its own Host.
            let (env, video, state): (EnvFn, VideoFn, StateFn) = match slot {
                BASELINE => (env_cb::<0>, video_cb::<0>, input_state_cb::<0>),
                _ => (env_cb::<1>, video_cb::<1>, input_state_cb::<1>),
            };
            set_environment(env);
            init();
            set_video(video);
            set_audio(audio_cb);
            set_audio_batch(audio_batch_cb);
            set_poll(input_poll_cb);
            set_state(state);

            let unknown: Vec<String> = with_host(slot, |h| {
                h.option_overrides.keys().filter(|k| !h.option_defs.contains_key(*k)).cloned().collect()
            })
            .unwrap_or_default();
            if !unknown.is_empty() {
                eprintln!("warning: core does not declare these options: {}", unknown.join(", "));
            }

            let connector = with_host(slot, |h| h.proc_iface).flatten().and_then(|gpa| {
                let get = |name: &str| -> ProcAddress {
                    let c = CString::new(name).unwrap();
                    gpa(c.as_ptr())
                };
                let version: unsafe extern "C" fn() -> u32 = std::mem::transmute(get("dj_api_version")?);
                if version() != 1 {
                    eprintln!("warning: coverage connector API version {} is not supported", version());
                    return None;
                }
                Some(Connector {
                    region_count: std::mem::transmute(get("dj_region_count")?),
                    get_region: std::mem::transmute(get("dj_get_region")?),
                    clear: std::mem::transmute(get("dj_clear")?),
                    set_tick_exec: std::mem::transmute(get("dj_set_tick_exec")?),
                    get_tick_count: std::mem::transmute(get("dj_get_tick_count")?),
                    trace: (|| {
                        Some(TraceFns {
                            start: std::mem::transmute(get("dj_trace_start")?),
                            stop: std::mem::transmute(get("dj_trace_stop")?),
                            read: std::mem::transmute(get("dj_trace_read")?),
                        })
                    })(),
                })
            });

            Ok(Core { _lib: lib, slot, api, library_name, library_version, connector, game_loaded: false })
        }
    }

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
            eprintln!("warning: {e:#}");
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

    pub fn run_frame(&mut self, capture: bool) -> Option<Frame> {
        let run = self.api.run;
        self.run_frame_with(capture, || unsafe { run() }).0
    }

    /// Runs one frame through `tick` instead of `retro_run` (a connector's `dj_run`, which
    /// calls it itself), capturing the picture as `run_frame` does.
    pub fn run_frame_with<T>(&mut self, capture: bool, tick: impl FnOnce() -> T) -> (Option<Frame>, T) {
        with_host(self.slot, |h| {
            h.capture_next_frame = capture;
            if capture {
                h.last_frame = None;
            }
        });
        self.gl_enter();
        let out = tick();
        self.gl_leave();
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
        unsafe { (self.api.set_controller_port_device)(port, device) }
    }

    pub fn serialize(&mut self) -> Result<Vec<u8>> {
        self.gl_enter();
        let r = self.serialize_inner();
        self.gl_leave();
        r
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
        self.gl_enter();
        let ok = unsafe { (self.api.unserialize)(state.as_ptr() as *const c_void, state.len()) };
        self.gl_leave();
        if !ok {
            bail!("core failed to load state");
        }
        Ok(())
    }

    pub fn memory(&mut self, id: u32) -> Option<&mut [u8]> {
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
    pub fn bus_byte(&mut self, addr: usize) -> Option<*mut u8> {
        with_host(self.slot, |h| {
            for d in &h.bus {
                if d.ptr == 0 {
                    continue;
                }
                let hit = if d.select != 0 { (addr & d.select) == (d.start & d.select) } else { addr >= d.start && addr < d.start + d.len.max(1) };
                if !hit {
                    continue;
                }
                let mut a = (addr - d.start) & !d.disconnect;
                if d.len > 0 {
                    a %= d.len;
                }
                return Some((d.ptr + d.offset + a) as *mut u8);
            }
            None
        })
        .flatten()
    }

    pub fn coverage_clear(&mut self) {
        if let Some(c) = &self.connector {
            unsafe { (c.clear)() }
        }
    }

    pub fn coverage_regions(&mut self) -> Vec<CoverageRegion> {
        let Some(c) = &self.connector else { return vec![] };
        let mut out = vec![];
        unsafe {
            for i in 0..(c.region_count)() {
                let mut r = DjRegionC { name: std::ptr::null(), flags: std::ptr::null_mut(), size: 0 };
                if !(c.get_region)(i, &mut r) {
                    continue;
                }
                let name = if r.name.is_null() { format!("R{i}") } else { CStr::from_ptr(r.name).to_string_lossy().into_owned() };
                let flags = if r.flags.is_null() { vec![] } else { std::slice::from_raw_parts(r.flags, r.size).to_vec() };
                out.push(CoverageRegion { name, flags });
            }
        }
        out
    }

    /// The connector's live flags for region `i`, without copying (for per-frame scans).
    pub fn coverage_slice(&mut self, i: usize) -> Option<&[u8]> {
        let c = self.connector.as_ref()?;
        unsafe {
            let mut r = DjRegionC { name: std::ptr::null(), flags: std::ptr::null_mut(), size: 0 };
            if !(c.get_region)(i as _, &mut r) || r.flags.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(r.flags, r.size))
        }
    }

    pub fn coverage_region_index(&mut self, name: &str) -> Option<usize> {
        let Some(c) = &self.connector else { return None };
        unsafe {
            for i in 0..(c.region_count)() {
                let mut r = DjRegionC { name: std::ptr::null(), flags: std::ptr::null_mut(), size: 0 };
                if (c.get_region)(i, &mut r) && !r.name.is_null() && CStr::from_ptr(r.name).to_string_lossy() == name {
                    return Some(i);
                }
            }
        }
        None
    }

    pub fn set_tick_exec(&mut self, region: Option<usize>, offset: u64) {
        if let Some(c) = &self.connector {
            unsafe { (c.set_tick_exec)(region.map(|r| r as i32).unwrap_or(-1), offset as i64) }
        }
    }

    pub fn can_trace(&self) -> bool {
        self.connector.as_ref().is_some_and(|c| c.trace.is_some())
    }

    /// Starts recording a trace of up to `capacity` events (instructions and data accesses).
    pub fn trace_start(&mut self, capacity: usize) -> Result<()> {
        let t = self.connector.as_ref().and_then(|c| c.trace.as_ref()).ok_or_else(|| anyhow!("this core's connector has no trace hook (dj_trace_start)"))?;
        unsafe { (t.start)(capacity) };
        Ok(())
    }

    /// Events recorded so far.
    pub fn trace_len(&mut self) -> usize {
        let Some(t) = self.connector.as_ref().and_then(|c| c.trace.as_ref()) else { return 0 };
        unsafe { (t.read)(std::ptr::null_mut(), std::ptr::null_mut()) }
    }

    /// Stops recording and returns the events, and how many were dropped past the capacity.
    pub fn trace_take(&mut self) -> (Vec<TraceEvent>, u64) {
        let Some(t) = self.connector.as_ref().and_then(|c| c.trace.as_ref()) else { return (vec![], 0) };
        unsafe {
            (t.stop)();
            let mut p: *const TraceEvent = std::ptr::null();
            let mut dropped = 0u64;
            let n = (t.read)(&mut p, &mut dropped);
            let v = if p.is_null() { vec![] } else { std::slice::from_raw_parts(p, n).to_vec() };
            (t.start)(0);
            (v, dropped)
        }
    }

    /// The connector's live flags for region `i`, writable: lets a caller clear a byte to see
    /// whether it runs again.
    pub fn coverage_slice_mut(&mut self, i: usize) -> Option<&mut [u8]> {
        let c = self.connector.as_ref()?;
        unsafe {
            let mut r = DjRegionC { name: std::ptr::null(), flags: std::ptr::null_mut(), size: 0 };
            if !(c.get_region)(i as _, &mut r) || r.flags.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts_mut(r.flags, r.size))
        }
    }

    pub fn tick_count(&mut self) -> Option<u64> {
        self.connector.as_ref().map(|c| unsafe { (c.get_tick_count)() })
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
        CORE_LOADED[self.slot].store(false, Ordering::SeqCst);
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
