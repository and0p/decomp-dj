//! The C side: libretro's environment commands and structs, the callbacks' types, and the
//! structs of dj's connector interface.

use super::*;

pub(super) const ENV_EXPERIMENTAL: u32 = 0x10000;

pub(super) const ENV_GET_CAN_DUPE: u32 = 3;

pub(super) const ENV_GET_SYSTEM_DIRECTORY: u32 = 9;

pub(super) const ENV_SET_PIXEL_FORMAT: u32 = 10;

pub(super) const ENV_SET_INPUT_DESCRIPTORS: u32 = 11;

pub(super) const ENV_GET_VARIABLE: u32 = 15;

pub(super) const ENV_SET_VARIABLES: u32 = 16;

pub(super) const ENV_GET_VARIABLE_UPDATE: u32 = 17;

pub(super) const ENV_SET_SUPPORT_NO_GAME: u32 = 18;

pub(super) const ENV_GET_SAVE_DIRECTORY: u32 = 31;

pub(super) const ENV_GET_LOG_INTERFACE: u32 = 27;

pub(super) const ENV_SET_SYSTEM_AV_INFO: u32 = 32;

pub(super) const ENV_SET_PROC_ADDRESS_CALLBACK: u32 = 33;

pub(super) const ENV_SET_CONTROLLER_INFO: u32 = 35;

pub(super) const ENV_SET_MEMORY_MAPS: u32 = 36 | ENV_EXPERIMENTAL;

pub(super) const ENV_SET_GEOMETRY: u32 = 37;

pub(super) const ENV_GET_LANGUAGE: u32 = 39;

pub(super) const ENV_SET_SERIALIZATION_QUIRKS: u32 = 44;

pub(super) const ENV_GET_AUDIO_VIDEO_ENABLE: u32 = 47 | ENV_EXPERIMENTAL;

pub(super) const ENV_GET_FASTFORWARDING: u32 = 49 | ENV_EXPERIMENTAL;

pub(super) const ENV_GET_CORE_OPTIONS_VERSION: u32 = 52;

pub(super) const ENV_SET_CORE_OPTIONS_DISPLAY: u32 = 55;

pub(super) const ENV_GET_INPUT_MAX_USERS: u32 = 61;

pub(super) const ENV_SET_HW_RENDER: u32 = 14;

pub(super) const ENV_GET_HW_RENDER_INTERFACE: u32 = 41 | ENV_EXPERIMENTAL;

pub(super) const ENV_SET_HW_SHARED_CONTEXT: u32 = 44 | ENV_EXPERIMENTAL;

pub(super) const ENV_GET_PREFERRED_HW_RENDER: u32 = 56;

pub(super) const HW_CONTEXT_OPENGL: u32 = 1;

pub(super) const HW_CONTEXT_OPENGL_CORE: u32 = 3;

/// `RETRO_HW_FRAME_BUFFER_VALID`: the frame is in the framebuffer, not in memory.
pub(super) const HW_FRAME_BUFFER_VALID: usize = usize::MAX;

pub const MEMORY_SAVE_RAM: u32 = 0;

pub const MEMORY_RTC: u32 = 1;

pub const MEMORY_SYSTEM_RAM: u32 = 2;

pub const MEMORY_VIDEO_RAM: u32 = 3;

pub(super) const DEVICE_ID_JOYPAD_MASK: u32 = 256;

#[repr(C)]
pub(super) struct GameInfo {
    pub(super) path: *const c_char,
    pub(super) data: *const c_void,
    pub(super) size: usize,
    pub(super) meta: *const c_char,
}

#[repr(C)]
pub(super) struct SystemInfo {
    pub(super) library_name: *const c_char,
    pub(super) library_version: *const c_char,
    pub(super) valid_extensions: *const c_char,
    pub(super) need_fullpath: bool,
    pub(super) block_extract: bool,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(super) struct GameGeometry {
    pub(super) base_width: u32,
    pub(super) base_height: u32,
    pub(super) max_width: u32,
    pub(super) max_height: u32,
    pub(super) aspect_ratio: f32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(super) struct SystemTiming {
    pub(super) fps: f64,
    pub(super) sample_rate: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(super) struct SystemAvInfo {
    pub(super) geometry: GameGeometry,
    pub(super) timing: SystemTiming,
}

#[repr(C)]
pub(super) struct Variable {
    pub(super) key: *const c_char,
    pub(super) value: *const c_char,
}

pub(super) type ProcAddress = Option<unsafe extern "C" fn()>;

#[repr(C)]
pub(super) struct GetProcAddressInterface {
    pub(super) get_proc_address: Option<unsafe extern "C" fn(*const c_char) -> ProcAddress>,
}

/// `struct retro_input_descriptor`: what the core calls one input. A list ends at a null
/// description.
#[repr(C)]
pub(super) struct InputDescriptor {
    pub(super) port: u32,
    pub(super) device: u32,
    pub(super) index: u32,
    pub(super) id: u32,
    pub(super) description: *const c_char,
}

#[repr(C)]
pub(super) struct MemoryDescriptor {
    pub(super) flags: u64,
    pub(super) ptr: *mut c_void,
    pub(super) offset: usize,
    pub(super) start: usize,
    pub(super) select: usize,
    pub(super) disconnect: usize,
    pub(super) len: usize,
    pub(super) addrspace: *const c_char,
}

#[repr(C)]
pub(super) struct MemoryMap {
    pub(super) descriptors: *const MemoryDescriptor,
    pub(super) num_descriptors: u32,
}

/// `struct dj_region`, API 2 (docs/connectors.md).
#[repr(C)]
pub(super) struct DjRegionC {
    pub(super) name: *const c_char,
    pub(super) flags: *mut u8,
    pub(super) size: usize,
    pub(super) cpu: *const c_char,
    pub(super) data: *const u8,
    pub(super) file_offset: i64,
}

impl DjRegionC {
    pub(super) fn empty() -> Self {
        DjRegionC { name: std::ptr::null(), flags: std::ptr::null_mut(), size: 0, cpu: std::ptr::null(), data: std::ptr::null(), file_offset: -1 }
    }
}

/// The connector API version the player speaks. No compatibility with older connectors:
/// rebuild them (connectors/README.md). Version 3: breakpoints (`dj_set_breakpoints`) in
/// place of the tick counter, and `dj_read_registers`.
pub const DJ_API_VERSION: u32 = 3;

/// `struct dj_break` (docs/connectors.md): where a breakpoint is.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct DjBreakC {
    pub(super) region: i32,
    pub(super) offset: i64,
}

/// `dj_break_fn`: called by the connector when an instruction at a breakpoint is about to
/// run, inside `retro_run`. Returning non-zero switches that breakpoint off until the list
/// is set again.
pub(super) type BreakFn = unsafe extern "C" fn(*mut c_void, i32, i64) -> i32;

pub(super) type HwResetFn = unsafe extern "C" fn();

/// `struct retro_hw_render_callback`.
#[repr(C)]
pub(super) struct HwRenderCallback {
    pub(super) context_type: u32,
    pub(super) context_reset: Option<HwResetFn>,
    pub(super) get_current_framebuffer: Option<unsafe extern "C" fn() -> usize>,
    pub(super) get_proc_address: Option<unsafe extern "C" fn(*const c_char) -> ProcAddress>,
    pub(super) depth: bool,
    pub(super) stencil: bool,
    pub(super) bottom_left_origin: bool,
    pub(super) version_major: u32,
    pub(super) version_minor: u32,
    pub(super) cache_context: bool,
    pub(super) context_destroy: Option<HwResetFn>,
    pub(super) debug_context: bool,
}

pub(super) type EnvFn = unsafe extern "C" fn(u32, *mut c_void) -> bool;

pub(super) type VideoFn = unsafe extern "C" fn(*const c_void, u32, u32, usize);

pub(super) type AudioFn = unsafe extern "C" fn(i16, i16);

pub(super) type AudioBatchFn = unsafe extern "C" fn(*const i16, usize) -> usize;

pub(super) type PollFn = unsafe extern "C" fn();

pub(super) type StateFn = unsafe extern "C" fn(u32, u32, u32, u32) -> i16;
