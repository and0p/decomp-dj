//! What the frontend keeps for the core in each slot, and the callbacks the core calls:
//! the environment, video, audio and input.

use super::*;

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
pub(super) struct Hw {
    pub(super) req: crate::gl::Request,
    pub(super) reset: Option<HwResetFn>,
    pub(super) destroy: Option<HwResetFn>,
    pub(super) bottom_left_origin: bool,
    /// Made once the game is loaded.
    pub(super) ctx: Option<crate::gl::GlContext>,
}

pub(super) struct Host {
    pub(super) system_dir: CString,
    pub(super) save_dir: CString,
    pub(super) pixel_format: PixelFormat,
    /// Option key -> (default value, allowed values)
    pub(super) option_defs: BTreeMap<String, (String, Vec<String>)>,
    pub(super) option_overrides: BTreeMap<String, String>,
    pub(super) option_cstrings: HashMap<String, CString>,
    pub(super) options_dirty: bool,
    pub(super) inputs: HashMap<InputKey, i16>,
    pub(super) capture_next_frame: bool,
    pub(super) last_frame: Option<Frame>,
    pub(super) proc_iface: Option<unsafe extern "C" fn(*const c_char) -> ProcAddress>,
    pub(super) bus: Vec<BusRange>,
    pub(super) av_info: SystemAvInfo,
    pub(super) hw: Option<Hw>,
    /// A GPU context the core asked for and was refused, and why.
    pub(super) hw_refused: Option<String>,
    /// The core's `retro_get_memory_data` and `retro_get_memory_size`, for a write made from
    /// inside its frame.
    pub(super) memory: Option<(unsafe extern "C" fn(u32) -> *mut c_void, unsafe extern "C" fn(u32) -> usize)>,
    /// Tell the core whether a picture is wanted (`GET_AUDIO_VIDEO_ENABLE`), so a core with
    /// costly graphics can skip drawing. Off: video is always said to be on.
    pub(super) video_hint: bool,
    /// What the core calls its inputs (`SET_INPUT_DESCRIPTORS`), as it last said.
    pub(super) input_names: Vec<(InputKey, String)>,
}

/// How many cores may be loaded at once.
pub const SLOTS: usize = 2;

/// The slot of the project's own core.
pub const BASELINE: usize = 0;

/// The slot of a comparison core.
pub const COMPARISON: usize = 1;

thread_local! {
    pub(super) static HOST: RefCell<[Option<Host>; SLOTS]> = const { RefCell::new([None, None]) };
}

pub(super) static CORE_LOADED: [AtomicBool; SLOTS] = [AtomicBool::new(false), AtomicBool::new(false)];

/// Where each slot's `retro_run` lives, while a core is loaded there (0: none): two slots
/// with the same address would be one library instance shared by two cores.
pub(super) static CORE_RUN: [std::sync::atomic::AtomicUsize; SLOTS] = [std::sync::atomic::AtomicUsize::new(0), std::sync::atomic::AtomicUsize::new(0)];

pub(super) fn with_host<R>(slot: usize, f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| h.borrow_mut()[slot].as_mut().map(f))
}

/// Parses a v0 core option string: "Description; default|other|other".
pub(super) fn parse_option_values(v: &str) -> (String, Vec<String>) {
    let list = v.split_once("; ").map(|(_, l)| l).unwrap_or(v);
    let values: Vec<String> = list.split('|').map(|s| s.to_string()).collect();
    (values.first().cloned().unwrap_or_default(), values)
}

/// `retro_log_printf_t`, without the variadic part: stable Rust can't define a C variadic
/// function, but the fixed arguments arrive the same way (registers on arm64 and x86-64), so
/// this reads the level and the format string and ignores the rest. Info, warnings and errors
/// go to the console log, warnings and errors also to stderr, with the format unexpanded. Some cores call the log callback without checking
/// it (mupen64plus-libretro-nx does in `update_variables`), so offering one also keeps them
/// from crashing.
pub(super) unsafe extern "C" fn core_log(level: u32, fmt: *const c_char) {
    if fmt.is_null() || level == 0 {
        return; // debug messages: too many
    }
    let text = CStr::from_ptr(fmt).to_string_lossy();
    // Info and up go to the console (View > Debug console in dj-gui); warnings and errors to stderr too.
    let lvl = match level {
        1 => crate::console::Level::Info,
        2 => crate::console::Level::Warn,
        _ => crate::console::Level::Error,
    };
    crate::console::push(lvl, "core", &text);
    if level >= 2 {
        eprint!("core {}: {}", if level >= 3 { "error" } else { "warning" }, text);
        if !text.ends_with('\n') {
            eprintln!();
        }
    }
}

pub(super) unsafe extern "C" fn env_cb<const S: usize>(cmd: u32, data: *mut c_void) -> bool {
    match cmd {
        ENV_GET_LOG_INTERFACE => {
            *(data as *mut unsafe extern "C" fn(u32, *const c_char)) = core_log;
            true
        }
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
            // bit 0: video, bit 1: audio. Video is on so frames can be captured; a core that
            // asked for the hint is told whether this frame's picture is wanted.
            let wanted = with_host(S, |h| !h.video_hint || h.capture_next_frame).unwrap_or(true);
            *(data as *mut i32) = wanted as i32;
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
        ENV_SET_INPUT_DESCRIPTORS => {
            let mut names = vec![];
            let mut d = data as *const InputDescriptor;
            // The list ends at a null description; the cap is for a core that forgets it.
            while !d.is_null() && !(*d).description.is_null() && names.len() < 4096 {
                let key = InputKey { port: (*d).port, device: (*d).device & 0xff, index: (*d).index, id: (*d).id };
                names.push((key, CStr::from_ptr((*d).description).to_string_lossy().into_owned()));
                d = d.add(1);
            }
            with_host(S, |h| h.input_names = names);
            true
        }
        ENV_SET_CONTROLLER_INFO | ENV_SET_SUPPORT_NO_GAME | ENV_SET_CORE_OPTIONS_DISPLAY | ENV_SET_SERIALIZATION_QUIRKS => true,
        _ => false,
    }
}

pub(super) fn hw_context_name(t: u32) -> String {
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

pub(super) unsafe extern "C" fn hw_framebuffer_cb<const S: usize>() -> usize {
    with_host(S, |h| h.hw.as_ref().and_then(|hw| hw.ctx.as_ref()).map_or(0, |c| c.framebuffer() as usize)).unwrap_or(0)
}

pub(super) unsafe extern "C" fn hw_proc_cb<const S: usize>(name: *const c_char) -> ProcAddress {
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
pub(super) fn scale(v: u16, max: u16) -> u8 {
    ((v & max) as u32 * 255 / max as u32) as u8
}

pub(super) unsafe extern "C" fn video_cb<const S: usize>(data: *const c_void, width: u32, height: u32, pitch: usize) {
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

pub(super) unsafe extern "C" fn audio_cb(_l: i16, _r: i16) {}

pub(super) unsafe extern "C" fn audio_batch_cb(_data: *const i16, frames: usize) -> usize {
    frames
}

pub(super) unsafe extern "C" fn input_poll_cb() {}

pub(super) unsafe extern "C" fn input_state_cb<const S: usize>(port: u32, device: u32, index: u32, id: u32) -> i16 {
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
